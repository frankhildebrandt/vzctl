package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"os/exec"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"
)

const (
	protocolVersion          = 1
	defaultPort              = 21950
	defaultToken             = "/var/lib/vzctl/agent.token"
	maxFrameSize             = 1_048_576
	maxIDBytes               = 128
	authFailureWait          = 250 * time.Millisecond
	maxExecTimeout           = 600 * time.Second
	defaultExecTime          = 30 * time.Second
	maxExecStream            = 256 * 1024
	maxExecStdin             = 256 * 1024
	maxExecArgs              = 256
	maxExecArgBytes          = 256 * 1024
	maxExecEnv               = 128
	maxExecEnvBytes          = 64 * 1024
	defaultTimeHintThreshold = time.Second
)

var (
	version   = "dev"
	startedAt = time.Now()
)

var capabilities = []string{"ping", "version", "exec", "exec_tty", "report_ip", "health", "time_hint", "fs_mount", "ca_inject", "network_probe"}

type request struct {
	V      int             `json:"v"`
	ID     string          `json:"id"`
	Method string          `json:"method"`
	Params json.RawMessage `json:"params"`
}

type response struct {
	V      int        `json:"v"`
	ID     string     `json:"id"`
	OK     bool       `json:"ok"`
	Result any        `json:"result,omitempty"`
	Error  *wireError `json:"error,omitempty"`
}

type wireError struct {
	Code    string         `json:"code"`
	Message string         `json:"message"`
	Details map[string]any `json:"details,omitempty"`
}

type server struct {
	token    []byte
	timeHint timeHintPolicy
}

type timeHintPolicy struct {
	thresholdMS int64
	dryRun      bool
	now         func() time.Time
	step        func(time.Time) error
}

type responseWriter struct {
	conn net.Conn
	mu   sync.Mutex
}

type execParams struct {
	Cmd       []string          `json:"cmd"`
	Cwd       string            `json:"cwd,omitempty"`
	Env       map[string]string `json:"env,omitempty"`
	StdinB64  *string           `json:"stdin_b64,omitempty"`
	TimeoutMS *int64            `json:"timeout_ms,omitempty"`
	TTY       bool              `json:"tty,omitempty"`
	Cols      *int              `json:"cols,omitempty"`
	Rows      *int              `json:"rows,omitempty"`
}

type cappedBuffer struct {
	buffer    bytes.Buffer
	limit     int
	truncated bool
}

func (w *cappedBuffer) Write(payload []byte) (int, error) {
	remaining := w.limit - w.buffer.Len()
	if remaining > 0 {
		keep := len(payload)
		if keep > remaining {
			keep = remaining
		}
		_, _ = w.buffer.Write(payload[:keep])
	}
	if len(payload) > remaining {
		w.truncated = true
	}
	return len(payload), nil
}

func main() {
	port := flag.Uint("port", defaultPort, "virtio-vsock listen port")
	tokenPath := flag.String("token-file", defaultToken, "authentication token file")
	timeHintThreshold := flag.Duration(
		"time-hint-threshold",
		defaultTimeHintThreshold,
		"minimum absolute clock offset before stepping",
	)
	timeHintDryRun := flag.Bool(
		"time-hint-dry-run",
		false,
		"measure time hints without changing the guest clock",
	)
	showVersion := flag.Bool("version", false, "print version and exit")
	flag.Parse()

	if *showVersion {
		fmt.Println(version)
		return
	}
	if *port == 0 || *port > 65535 {
		log.Fatal("invalid vsock port")
	}
	if *timeHintThreshold < time.Millisecond {
		log.Fatal("time-hint threshold must be at least 1ms")
	}

	token, err := loadToken(*tokenPath)
	if err != nil {
		log.Fatalf("cannot load authentication token: %v", err)
	}

	listener, err := listenVsock(uint32(*port))
	if err != nil {
		log.Fatalf("cannot listen on virtio-vsock port %d: %v", *port, err)
	}
	defer listener.Close()

	log.Printf("vzctl-agent %s listening on virtio-vsock port %d", version, *port)
	s := &server{
		token: token,
		timeHint: timeHintPolicy{
			thresholdMS: timeHintThreshold.Milliseconds(),
			dryRun:      *timeHintDryRun,
			now:         time.Now,
			step:        setSystemClock,
		},
	}
	var connectionID atomic.Uint64
	for {
		conn, err := listener.Accept()
		if err != nil {
			if errors.Is(err, net.ErrClosed) {
				return
			}
			log.Printf("accept failed")
			continue
		}
		id := connectionID.Add(1)
		go func() {
			defer conn.Close()
			if err := s.serveConn(conn); err != nil && !errors.Is(err, io.EOF) {
				// Only the connection identifier is logged. Requests and tokens are not.
				log.Printf("connection %d closed: protocol or I/O error", id)
			}
		}()
	}
}

func loadToken(path string) ([]byte, error) {
	info, err := os.Stat(path)
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() {
		return nil, errors.New("token path is not a regular file")
	}
	if info.Mode().Perm() != 0o600 {
		return nil, fmt.Errorf("token file mode must be 0600, got %04o", info.Mode().Perm())
	}
	if info.Size() > 4096 {
		return nil, errors.New("token file is too large")
	}

	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	token := strings.TrimSpace(string(raw))
	if token == "" || strings.ContainsAny(token, " \t\r\n") {
		return nil, errors.New("token file has invalid content")
	}
	decoded, err := base64.RawURLEncoding.DecodeString(token)
	if err != nil || len(decoded) < 32 {
		return nil, errors.New("token must be unpadded base64url with at least 256 bits")
	}
	return []byte(token), nil
}

func (s *server) serveConn(conn net.Conn) error {
	payload, err := readFrame(conn)
	if err != nil {
		return err
	}
	hello, reqErr := decodeRequest(payload)
	if reqErr != nil {
		if hello.ID != "" {
			_ = writeResponse(conn, errorResponse(hello.ID, "proto", reqErr.Error(), nil))
		}
		return reqErr
	}
	if hello.Method != "hello" {
		_ = writeResponse(conn, errorResponse(hello.ID, "auth", "hello is required before commands", nil))
		return errors.New("command before hello")
	}
	ok, closeConn := s.handleHello(conn, hello)
	if closeConn || !ok {
		return errors.New("hello rejected")
	}

	connectionContext, cancelConnection := context.WithCancel(context.Background())
	defer cancelConnection()
	writer := &responseWriter{conn: conn}
	inflight := make(map[string]context.CancelFunc)
	var inflightMu sync.Mutex
	var workers sync.WaitGroup
	defer func() {
		cancelConnection()
		inflightMu.Lock()
		for _, cancel := range inflight {
			cancel()
		}
		inflightMu.Unlock()
		workers.Wait()
	}()

	for {
		payload, err = readFrame(conn)
		if err != nil {
			return err
		}
		req, reqErr := decodeRequest(payload)
		if reqErr != nil {
			if req.ID != "" {
				_ = writer.write(errorResponse(req.ID, "proto", reqErr.Error(), nil))
			}
			return reqErr
		}
		if req.Method == "hello" {
			if err := writer.write(errorResponse(req.ID, "proto", "hello is only valid as the first frame", nil)); err != nil {
				return err
			}
			continue
		}
		if req.Method == "cancel" {
			response := handleCancel(req, &inflightMu, inflight)
			if err := writer.write(response); err != nil {
				return err
			}
			continue
		}

		if req.Method == "exec" {
			var peek execParams
			if err := decodeParams(req.Params, &peek); err == nil && peek.TTY {
				inflightMu.Lock()
				if _, exists := inflight[req.ID]; exists {
					inflightMu.Unlock()
					if err := writer.write(errorResponse(req.ID, "proto", "request id is already in flight", nil)); err != nil {
						return err
					}
					continue
				}
				requestContext, cancelRequest := context.WithCancel(connectionContext)
				inflight[req.ID] = cancelRequest
				inflightMu.Unlock()
				response, session := prepareTTYExec(requestContext, req)
				if err := writer.write(response); err != nil {
					cancelRequest()
					inflightMu.Lock()
					delete(inflight, req.ID)
					inflightMu.Unlock()
					return err
				}
				if session == nil {
					cancelRequest()
					inflightMu.Lock()
					delete(inflight, req.ID)
					inflightMu.Unlock()
					continue
				}
				err := runTTYSession(conn, writer, session)
				cancelRequest()
				inflightMu.Lock()
				delete(inflight, req.ID)
				inflightMu.Unlock()
				return err
			}
		}

		inflightMu.Lock()
		if _, exists := inflight[req.ID]; exists {
			inflightMu.Unlock()
			if err := writer.write(errorResponse(req.ID, "proto", "request id is already in flight", nil)); err != nil {
				return err
			}
			continue
		}
		requestContext, cancelRequest := context.WithCancel(connectionContext)
		inflight[req.ID] = cancelRequest
		inflightMu.Unlock()

		workers.Add(1)
		go func() {
			defer workers.Done()
			response := handleRequestWithPolicy(requestContext, req, s.timeHint)
			inflightMu.Lock()
			_ = writer.write(response)
			delete(inflight, req.ID)
			inflightMu.Unlock()
		}()
	}
}

func (w *responseWriter) write(response response) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	return writeResponse(w.conn, response)
}

func (w *responseWriter) writeMux(frameType byte, payload []byte) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	return writeMuxFrame(w.conn, frameType, payload)
}

func (s *server) handleHello(conn net.Conn, req request) (bool, bool) {
	if req.V != protocolVersion {
		_ = writeResponse(conn, errorResponse(req.ID, "proto", "unsupported protocol version", map[string]any{
			"supported_versions": []int{protocolVersion},
		}))
		return false, true
	}
	var params struct {
		Token         string `json:"token"`
		HelperVersion string `json:"helper_version"`
	}
	if err := decodeParams(req.Params, &params); err != nil || params.Token == "" {
		_ = writeResponse(conn, errorResponse(req.ID, "proto", "invalid hello parameters", nil))
		return false, true
	}
	providedTokenHash := sha256.Sum256([]byte(params.Token))
	expectedTokenHash := sha256.Sum256(s.token)
	if subtle.ConstantTimeCompare(providedTokenHash[:], expectedTokenHash[:]) != 1 {
		_ = writeResponse(conn, errorResponse(req.ID, "auth", "authentication failed", nil))
		time.Sleep(authFailureWait)
		return false, true
	}
	_ = writeResponse(conn, successResponse(req.ID, versionResult()))
	return true, false
}

func handleCancel(req request, mu *sync.Mutex, inflight map[string]context.CancelFunc) response {
	var params struct {
		ID string `json:"id"`
	}
	if err := decodeParams(req.Params, &params); err != nil || params.ID == "" {
		return errorResponse(req.ID, "proto", "invalid cancel parameters", nil)
	}
	mu.Lock()
	cancel, found := inflight[params.ID]
	mu.Unlock()
	if found {
		cancel()
	}
	return successResponse(req.ID, map[string]any{"cancelled": found})
}

func handleRequest(ctx context.Context, req request) response {
	return handleRequestWithPolicy(ctx, req, timeHintPolicy{})
}

func handleRequestWithPolicy(ctx context.Context, req request, policy timeHintPolicy) response {
	if req.V != protocolVersion {
		return errorResponse(req.ID, "proto", "unsupported protocol version", map[string]any{
			"supported_versions": []int{protocolVersion},
		})
	}
	switch req.Method {
	case "ping":
		var params struct {
			Nonce *string `json:"nonce"`
		}
		if err := decodeParams(req.Params, &params); err != nil {
			return errorResponse(req.ID, "proto", "invalid ping parameters", nil)
		}
		result := map[string]any{"pong": true}
		if params.Nonce != nil {
			result["nonce"] = *params.Nonce
		}
		return successResponse(req.ID, result)
	case "version":
		if err := decodeParams(req.Params, &struct{}{}); err != nil {
			return errorResponse(req.ID, "proto", "invalid version parameters", nil)
		}
		return successResponse(req.ID, versionResult())
	case "health":
		if err := decodeParams(req.Params, &struct{}{}); err != nil {
			return errorResponse(req.ID, "proto", "invalid health parameters", nil)
		}
		return successResponse(req.ID, map[string]any{
			"status":    "ok",
			"uptime_ms": time.Since(startedAt).Milliseconds(),
			"checks": map[string]any{
				"service":    map[string]any{"ok": true},
				"token_file": map[string]any{"ok": true},
			},
		})
	case "exec":
		return handleExec(ctx, req)
	case "report_ip":
		if err := decodeParams(req.Params, &struct{}{}); err != nil {
			return errorResponse(req.ID, "proto", "invalid report_ip parameters", nil)
		}
		interfaces, err := collectInterfaces()
		if err != nil {
			return errorResponse(req.ID, "internal", "cannot enumerate network interfaces", nil)
		}
		return successResponse(req.ID, map[string]any{"interfaces": interfaces})
	case "time_hint":
		return handleTimeHint(req, policy)
	case "fs.mount":
		return handleFSMount(req)
	case "fs.unmount":
		return handleFSUnmount(req)
	case "ca_inject":
		return handleCAInject(req)
	case "network_probe":
		return handleNetworkProbe(ctx, req)
	default:
		return errorResponse(req.ID, "unsupported", "method is not supported", map[string]any{
			"method": req.Method,
		})
	}
}

func handleTimeHint(req request, policy timeHintPolicy) response {
	var params struct {
		HostUnixMS int64  `json:"host_unix_ms"`
		Reason     string `json:"reason"`
	}
	if err := decodeParams(req.Params, &params); err != nil || params.HostUnixMS <= 0 {
		return errorResponse(req.ID, "proto", "invalid time_hint parameters", nil)
	}
	switch params.Reason {
	case "handshake", "wake", "manual":
	default:
		return errorResponse(req.ID, "proto", "invalid time_hint reason", nil)
	}

	policy = policy.normalized()
	observedGuestUnixMS := policy.now().UnixMilli()
	offsetMS := params.HostUnixMS - observedGuestUnixMS
	result := map[string]any{
		"observed_guest_unix_ms": observedGuestUnixMS,
		"offset_ms":              offsetMS,
		"action":                 "none",
	}
	if absoluteMilliseconds(offsetMS) <= policy.thresholdMS {
		return successResponse(req.ID, result)
	}
	if policy.dryRun {
		result["action"] = "skipped"
		return successResponse(req.ID, result)
	}
	if err := policy.step(time.UnixMilli(params.HostUnixMS)); err != nil {
		return errorResponse(req.ID, "internal", "cannot step guest clock", nil)
	}
	result["action"] = "stepped"
	return successResponse(req.ID, result)
}

func (p timeHintPolicy) normalized() timeHintPolicy {
	if p.thresholdMS == 0 {
		p.thresholdMS = defaultTimeHintThreshold.Milliseconds()
	}
	if p.now == nil {
		p.now = time.Now
	}
	if p.step == nil {
		p.step = setSystemClock
	}
	return p
}

func absoluteMilliseconds(value int64) int64 {
	if value < 0 {
		return -value
	}
	return value
}

func handleExec(parent context.Context, req request) response {
	var params execParams
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid exec parameters", nil)
	}
	if params.TTY {
		return errorResponse(req.ID, "proto", "tty exec must be handled on the connection", nil)
	}
	timeout, stdin, validationError := validateExecParams(params)
	if validationError != nil {
		return errorResponse(req.ID, "proto", validationError.Error(), nil)
	}

	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()
	if ctx.Err() != nil {
		return errorResponse(req.ID, "timeout", "request cancelled or deadline exceeded", map[string]any{
			"reason": "cancelled",
		})
	}
	command := exec.Command(params.Cmd[0], params.Cmd[1:]...)
	command.Dir = params.Cwd
	command.Env = sanitizedEnvironment(params.Env)
	command.Stdin = bytes.NewReader(stdin)
	command.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	stdout := &cappedBuffer{limit: maxExecStream}
	stderr := &cappedBuffer{limit: maxExecStream}
	command.Stdout = stdout
	command.Stderr = stderr

	if err := command.Start(); err != nil {
		return execFailure(req.ID, nil, nil, stdout, stderr, err.Error())
	}
	wait := make(chan error, 1)
	go func() { wait <- command.Wait() }()

	select {
	case err := <-wait:
		if err == nil {
			return successResponse(req.ID, map[string]any{
				"exit":      0,
				"stdout":    stdout.buffer.String(),
				"stderr":    stderr.buffer.String(),
				"truncated": stdout.truncated || stderr.truncated,
			})
		}
		exit, signal := processStatus(command.ProcessState)
		return execFailure(req.ID, exit, signal, stdout, stderr, err.Error())
	case <-ctx.Done():
		_ = syscall.Kill(-command.Process.Pid, syscall.SIGKILL)
		<-wait
		return errorResponse(req.ID, "timeout", "request cancelled or deadline exceeded", map[string]any{
			"reason": "cancelled",
		})
	}
}

type ttySession struct {
	command *exec.Cmd
	ptm     *os.File
	pts     *os.File
}

func prepareTTYExec(parent context.Context, req request) (response, *ttySession) {
	var params execParams
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid exec parameters", nil), nil
	}
	if !params.TTY {
		return errorResponse(req.ID, "proto", "tty is required", nil), nil
	}
	if params.StdinB64 != nil {
		return errorResponse(req.ID, "proto", "stdin_b64 is invalid with tty", nil), nil
	}
	if _, _, err := validateExecParams(params); err != nil {
		return errorResponse(req.ID, "proto", err.Error(), nil), nil
	}
	cols := uint16(80)
	rows := uint16(24)
	if params.Cols != nil {
		if *params.Cols <= 0 || *params.Cols > 65535 {
			return errorResponse(req.ID, "proto", "cols must be between 1 and 65535", nil), nil
		}
		cols = uint16(*params.Cols)
	}
	if params.Rows != nil {
		if *params.Rows <= 0 || *params.Rows > 65535 {
			return errorResponse(req.ID, "proto", "rows must be between 1 and 65535", nil), nil
		}
		rows = uint16(*params.Rows)
	}
	if parent.Err() != nil {
		return errorResponse(req.ID, "timeout", "request cancelled or deadline exceeded", map[string]any{
			"reason": "cancelled",
		}), nil
	}

	ptm, pts, err := openPTY()
	if err != nil {
		return errorResponse(req.ID, "unsupported", "tty exec is unavailable", map[string]any{
			"message": err.Error(),
		}), nil
	}
	if err := setPTYSize(ptm, cols, rows); err != nil {
		_ = ptm.Close()
		_ = pts.Close()
		return errorResponse(req.ID, "internal", "cannot set pty size", nil), nil
	}

	command := exec.Command(params.Cmd[0], params.Cmd[1:]...)
	command.Dir = params.Cwd
	command.Env = sanitizedEnvironment(params.Env)
	command.Stdin = pts
	command.Stdout = pts
	command.Stderr = pts
	// Ctty must be the child's FD number (stdin=0), not the parent's pts.Fd().
	command.SysProcAttr = &syscall.SysProcAttr{
		Setsid:  true,
		Setctty: true,
		Ctty:    0,
	}
	if err := command.Start(); err != nil {
		_ = ptm.Close()
		_ = pts.Close()
		return execFailure(req.ID, nil, nil, &cappedBuffer{limit: maxExecStream}, &cappedBuffer{limit: maxExecStream}, err.Error()), nil
	}
	_ = pts.Close()
	return successResponse(req.ID, map[string]any{"upgraded": true}), &ttySession{
		command: command,
		ptm:     ptm,
		pts:     pts,
	}
}

func runTTYSession(conn net.Conn, writer *responseWriter, session *ttySession) error {
	defer func() {
		_ = session.ptm.Close()
		if session.command.Process != nil {
			_ = syscall.Kill(-session.command.Process.Pid, syscall.SIGHUP)
			_ = syscall.Kill(-session.command.Process.Pid, syscall.SIGKILL)
			_, _ = session.command.Process.Wait()
		}
	}()

	done := make(chan struct{})
	var exitOnce sync.Once
	var exitStatus int32
	waitErr := make(chan error, 1)
	go func() {
		waitErr <- session.command.Wait()
	}()

	go func() {
		buf := make([]byte, 32*1024)
		for {
			n, err := session.ptm.Read(buf)
			if n > 0 {
				if writeErr := writer.writeMux(muxStdout, buf[:n]); writeErr != nil {
					exitOnce.Do(func() { close(done) })
					return
				}
			}
			if err != nil {
				return
			}
		}
	}()

	go func() {
		err := <-waitErr
		status := int32(0)
		if err != nil {
			if exit, _ := processStatus(session.command.ProcessState); exit != nil {
				status = int32(*exit)
			} else {
				status = 1
			}
		}
		exitOnce.Do(func() {
			exitStatus = status
			close(done)
		})
	}()

	go func() {
		for {
			frameType, payload, err := readMuxFrame(conn)
			if err != nil {
				exitOnce.Do(func() { close(done) })
				return
			}
			switch frameType {
			case muxStdin:
				if _, err := session.ptm.Write(payload); err != nil {
					exitOnce.Do(func() { close(done) })
					return
				}
			case muxStdinEOF:
				// Keep PTY open until process exits; ignore further stdin.
			case muxResize:
				if len(payload) != 4 {
					exitOnce.Do(func() { close(done) })
					return
				}
				cols := binary.LittleEndian.Uint16(payload[0:2])
				rows := binary.LittleEndian.Uint16(payload[2:4])
				_ = setPTYSize(session.ptm, cols, rows)
			default:
				exitOnce.Do(func() { close(done) })
				return
			}
		}
	}()

	<-done
	payload := make([]byte, 4)
	binary.LittleEndian.PutUint32(payload, uint32(exitStatus))
	_ = writer.writeMux(muxExit, payload)
	return nil
}

func validateExecParams(params execParams) (time.Duration, []byte, error) {
	if len(params.Cmd) == 0 || len(params.Cmd) > maxExecArgs {
		return 0, nil, fmt.Errorf("cmd must contain 1 to %d argv strings", maxExecArgs)
	}
	argBytes := 0
	for _, arg := range params.Cmd {
		if strings.ContainsRune(arg, 0) {
			return 0, nil, errors.New("cmd entries must not contain NUL")
		}
		argBytes += len(arg)
	}
	if argBytes > maxExecArgBytes {
		return 0, nil, errors.New("cmd exceeds 256 KiB")
	}
	if len(params.Env) > maxExecEnv {
		return 0, nil, fmt.Errorf("env exceeds %d entries", maxExecEnv)
	}
	envBytes := 0
	for key, value := range params.Env {
		if key == "" || strings.ContainsAny(key, "=\x00") || strings.ContainsRune(value, 0) {
			return 0, nil, errors.New("env contains an invalid key or value")
		}
		if key == "PATH" || strings.HasPrefix(key, "VZCTL_AGENT_") {
			return 0, nil, errors.New("env attempts to replace a protected variable")
		}
		envBytes += len(key) + len(value)
	}
	if envBytes > maxExecEnvBytes {
		return 0, nil, errors.New("env exceeds 64 KiB")
	}

	timeout := defaultExecTime
	if params.TimeoutMS != nil {
		if *params.TimeoutMS <= 0 || *params.TimeoutMS > maxExecTimeout.Milliseconds() {
			return 0, nil, errors.New("timeout_ms must be between 1 and 600000")
		}
		timeout = time.Duration(*params.TimeoutMS) * time.Millisecond
	}

	var stdin []byte
	if params.StdinB64 != nil {
		if len(*params.StdinB64) > (maxExecStdin+2)/3*4 {
			return 0, nil, errors.New("stdin_b64 exceeds 256 KiB decoded")
		}
		var err error
		stdin, err = base64.StdEncoding.DecodeString(*params.StdinB64)
		if err != nil {
			return 0, nil, errors.New("stdin_b64 is not valid base64")
		}
		if len(stdin) > maxExecStdin {
			return 0, nil, errors.New("stdin_b64 exceeds 256 KiB decoded")
		}
	}
	return timeout, stdin, nil
}

func sanitizedEnvironment(extra map[string]string) []string {
	values := map[string]string{
		"LANG": "C.UTF-8",
		"PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
	}
	for key, value := range extra {
		values[key] = value
	}
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	result := make([]string, 0, len(keys))
	for _, key := range keys {
		result = append(result, key+"="+values[key])
	}
	return result
}

func processStatus(state *os.ProcessState) (*int, *int) {
	if state == nil {
		return nil, nil
	}
	status, ok := state.Sys().(syscall.WaitStatus)
	if !ok {
		return nil, nil
	}
	if status.Exited() {
		exit := status.ExitStatus()
		return &exit, nil
	}
	if status.Signaled() {
		signal := int(status.Signal())
		return nil, &signal
	}
	return nil, nil
}

func execFailure(
	id string,
	exit *int,
	signal *int,
	stdout *cappedBuffer,
	stderr *cappedBuffer,
	message string,
) response {
	details := map[string]any{
		"stdout":    stdout.buffer.String(),
		"stderr":    stderr.buffer.String(),
		"truncated": stdout.truncated || stderr.truncated,
	}
	if exit == nil {
		details["exit"] = nil
	} else {
		details["exit"] = *exit
	}
	if signal != nil {
		details["signal"] = *signal
	}
	return errorResponse(id, "exec_failed", message, details)
}

func collectInterfaces() ([]map[string]any, error) {
	all, err := net.Interfaces()
	if err != nil {
		return nil, err
	}
	result := make([]map[string]any, 0, len(all))
	for _, iface := range all {
		if iface.Flags&net.FlagUp == 0 || iface.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}
		addresses := make([]string, 0, len(addrs))
		for _, addr := range addrs {
			ip, network, ok := parseAddress(addr)
			if ok && isGuestAddress(ip) {
				addresses = append(addresses, network)
			}
		}
		if len(addresses) == 0 {
			continue
		}
		sort.Strings(addresses)
		result = append(result, map[string]any{
			"name":      iface.Name,
			"mac":       iface.HardwareAddr.String(),
			"addresses": addresses,
		})
	}
	sort.Slice(result, func(i, j int) bool {
		return result[i]["name"].(string) < result[j]["name"].(string)
	})
	return result, nil
}

func parseAddress(addr net.Addr) (net.IP, string, bool) {
	ip, network, err := net.ParseCIDR(addr.String())
	if err != nil || ip == nil || network == nil {
		return nil, "", false
	}
	ones, _ := network.Mask.Size()
	return ip, fmt.Sprintf("%s/%d", ip.String(), ones), true
}

func isGuestAddress(ip net.IP) bool {
	if ip.IsLoopback() {
		return false
	}
	if v4 := ip.To4(); v4 != nil && v4[3] == 0 {
		return false
	}
	return true
}

func versionResult() map[string]any {
	return map[string]any{
		"v":             protocolVersion,
		"agent_version": version,
		"capabilities":  capabilities,
	}
}

func decodeRequest(payload []byte) (request, error) {
	var req request
	if err := decodeSingleJSON(payload, &req); err != nil {
		return req, errors.New("payload must be exactly one JSON object")
	}
	if req.V != protocolVersion && req.V != 0 {
		// A syntactically valid version is handled by the handshake/request path
		// so the response can include supported_versions.
	}
	if req.ID == "" || len([]byte(req.ID)) > maxIDBytes {
		return req, errors.New("id must contain 1 to 128 UTF-8 bytes")
	}
	if req.Method == "" || !isASCII(req.Method) {
		return req, errors.New("method must be non-empty ASCII")
	}
	if len(req.Params) == 0 {
		return req, errors.New("params must be an object")
	}
	var params map[string]json.RawMessage
	if err := decodeSingleJSON(req.Params, &params); err != nil || params == nil {
		return req, errors.New("params must be an object")
	}
	return req, nil
}

func decodeParams(raw json.RawMessage, dst any) error {
	return decodeSingleJSON(raw, dst)
}

func decodeSingleJSON(raw []byte, dst any) error {
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	if err := decoder.Decode(dst); err != nil {
		return err
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("trailing JSON data")
	}
	return nil
}

func isASCII(value string) bool {
	for _, r := range value {
		if r > 127 {
			return false
		}
	}
	return true
}

func successResponse(id string, result any) response {
	return response{V: protocolVersion, ID: id, OK: true, Result: result}
}

func errorResponse(id, code, message string, details map[string]any) response {
	return response{
		V:  protocolVersion,
		ID: id,
		OK: false,
		Error: &wireError{
			Code:    code,
			Message: message,
			Details: details,
		},
	}
}

func readFrame(r io.Reader) ([]byte, error) {
	var prefix [4]byte
	if _, err := io.ReadFull(r, prefix[:]); err != nil {
		return nil, err
	}
	length := binary.LittleEndian.Uint32(prefix[:])
	if length == 0 || length > maxFrameSize {
		return nil, errors.New("invalid frame length")
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(r, payload); err != nil {
		return nil, err
	}
	return payload, nil
}

func writeResponse(w io.Writer, resp response) error {
	payload, err := json.Marshal(resp)
	if err != nil {
		return err
	}
	var prefix [4]byte
	binary.LittleEndian.PutUint32(prefix[:], uint32(len(payload)))
	if err := writeAll(w, prefix[:]); err != nil {
		return err
	}
	return writeAll(w, payload)
}

func writeAll(w io.Writer, payload []byte) error {
	for len(payload) > 0 {
		n, err := w.Write(payload)
		if err != nil {
			return err
		}
		if n == 0 {
			return io.ErrShortWrite
		}
		payload = payload[n:]
	}
	return nil
}
