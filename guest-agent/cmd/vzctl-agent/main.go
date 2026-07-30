package main

import (
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
	"strings"
	"sync/atomic"
	"time"
)

const (
	protocolVersion = 1
	defaultPort     = 21950
	defaultToken    = "/run/vzctl/agent.token"
	maxFrameSize    = 1_048_576
	maxIDBytes      = 128
	authFailureWait = 250 * time.Millisecond
)

var (
	version   = "dev"
	startedAt = time.Now()
)

var capabilities = []string{"ping", "version", "health"}

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
	token []byte
}

func main() {
	port := flag.Uint("port", defaultPort, "virtio-vsock listen port")
	tokenPath := flag.String("token-file", defaultToken, "authentication token file")
	showVersion := flag.Bool("version", false, "print version and exit")
	flag.Parse()

	if *showVersion {
		fmt.Println(version)
		return
	}
	if *port == 0 || *port > 65535 {
		log.Fatal("invalid vsock port")
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
	s := &server{token: token}
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
	authenticated := false
	for {
		payload, err := readFrame(conn)
		if err != nil {
			return err
		}
		req, reqErr := decodeRequest(payload)
		if reqErr != nil {
			if req.ID != "" {
				_ = writeResponse(conn, errorResponse(req.ID, "proto", reqErr.Error(), nil))
			}
			return reqErr
		}

		if !authenticated {
			if req.Method != "hello" {
				_ = writeResponse(conn, errorResponse(req.ID, "auth", "hello is required before commands", nil))
				return errors.New("command before hello")
			}
			ok, closeConn := s.handleHello(conn, req)
			if closeConn {
				return errors.New("hello rejected")
			}
			authenticated = ok
			continue
		}
		if req.Method == "hello" {
			if err := writeResponse(conn, errorResponse(req.ID, "proto", "hello is only valid as the first frame", nil)); err != nil {
				return err
			}
			continue
		}
		if err := writeResponse(conn, handleRequest(req)); err != nil {
			return err
		}
	}
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

func handleRequest(req request) response {
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
	case "cancel":
		var params struct {
			ID string `json:"id"`
		}
		if err := decodeParams(req.Params, &params); err != nil || params.ID == "" {
			return errorResponse(req.ID, "proto", "invalid cancel parameters", nil)
		}
		return successResponse(req.ID, map[string]any{"cancelled": false})
	case "exec", "report_ip", "time_hint":
		return errorResponse(req.ID, "unsupported", "method is not implemented in the boot-proof slice", map[string]any{
			"method": req.Method,
		})
	default:
		return errorResponse(req.ID, "unsupported", "method is not supported", map[string]any{
			"method": req.Method,
		})
	}
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
