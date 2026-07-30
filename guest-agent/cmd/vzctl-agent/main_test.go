package main

import (
	"context"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestLoadTokenRequiresModeAndStrength(t *testing.T) {
	token := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	path := filepath.Join(t.TempDir(), "token")
	if err := os.WriteFile(path, []byte(token+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := loadToken(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != token {
		t.Fatalf("got %q", got)
	}
	if err := os.Chmod(path, 0o640); err != nil {
		t.Fatal(err)
	}
	if _, err := loadToken(path); err == nil {
		t.Fatal("expected insecure mode to fail")
	}
}

func TestHelloPingVersionHealthExecAndReportIP(t *testing.T) {
	token := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	client, agent := net.Pipe()
	defer client.Close()
	done := make(chan error, 1)
	go func() {
		done <- (&server{token: []byte(token)}).serveConn(agent)
	}()

	hello := exchange(t, client, map[string]any{
		"v": 1, "id": "hello-1", "method": "hello",
		"params": map[string]any{"token": token, "helper_version": "test"},
	})
	assertOK(t, hello)
	result := hello["result"].(map[string]any)
	caps := result["capabilities"].([]any)
	if len(caps) != 6 {
		t.Fatalf("capabilities = %#v", caps)
	}

	ping := exchange(t, client, map[string]any{
		"v": 1, "id": "ping-1", "method": "ping",
		"params": map[string]any{"nonce": "proof"},
	})
	assertOK(t, ping)
	if ping["result"].(map[string]any)["nonce"] != "proof" {
		t.Fatalf("ping result = %#v", ping)
	}

	for _, method := range []string{"version", "health"} {
		got := exchange(t, client, map[string]any{
			"v": 1, "id": method + "-1", "method": method, "params": map[string]any{},
		})
		assertOK(t, got)
	}

	execResult := exchange(t, client, map[string]any{
		"v": 1, "id": "exec-1", "method": "exec",
		"params": map[string]any{"cmd": []string{"/bin/sh", "-c", "printf out; printf err >&2"}},
	})
	assertOK(t, execResult)
	execPayload := execResult["result"].(map[string]any)
	if execPayload["exit"] != float64(0) || execPayload["stdout"] != "out" || execPayload["stderr"] != "err" {
		t.Fatalf("exec response = %#v", execResult)
	}

	reportIP := exchange(t, client, map[string]any{
		"v": 1, "id": "ip-1", "method": "report_ip", "params": map[string]any{},
	})
	assertOK(t, reportIP)
	if _, ok := reportIP["result"].(map[string]any)["interfaces"].([]any); !ok {
		t.Fatalf("report_ip response = %#v", reportIP)
	}

	_ = client.Close()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("server did not stop")
	}
}

func TestTimeHintNoneBelowThreshold(t *testing.T) {
	now := time.UnixMilli(1_785_387_600_000)
	req := request{
		V: 1, ID: "time-none", Method: "time_hint",
		Params: json.RawMessage(`{"host_unix_ms":1785387600500,"reason":"handshake"}`),
	}
	got := handleRequestWithPolicy(context.Background(), req, timeHintPolicy{
		thresholdMS: 1_000,
		now:         func() time.Time { return now },
		step: func(time.Time) error {
			t.Fatal("clock must not be stepped")
			return nil
		},
	})
	if !got.OK {
		t.Fatalf("response = %#v", got)
	}
	result := got.Result.(map[string]any)
	if result["observed_guest_unix_ms"] != int64(1_785_387_600_000) ||
		result["offset_ms"] != int64(500) || result["action"] != "none" {
		t.Fatalf("result = %#v", result)
	}
}

func TestTimeHintStepsAboveThreshold(t *testing.T) {
	now := time.UnixMilli(1_785_387_590_000)
	var steppedTo time.Time
	req := request{
		V: 1, ID: "time-step", Method: "time_hint",
		Params: json.RawMessage(`{"host_unix_ms":1785387600000,"reason":"wake"}`),
	}
	got := handleRequestWithPolicy(context.Background(), req, timeHintPolicy{
		thresholdMS: 1_000,
		now:         func() time.Time { return now },
		step:        func(value time.Time) error { steppedTo = value; return nil },
	})
	if !got.OK || got.Result.(map[string]any)["action"] != "stepped" {
		t.Fatalf("response = %#v", got)
	}
	if steppedTo.UnixMilli() != 1_785_387_600_000 {
		t.Fatalf("stepped to %d", steppedTo.UnixMilli())
	}
}

func TestTimeHintDryRunSkipsAndValidatesReason(t *testing.T) {
	now := time.UnixMilli(1_785_387_590_000)
	policy := timeHintPolicy{
		thresholdMS: 1_000,
		dryRun:      true,
		now:         func() time.Time { return now },
		step: func(time.Time) error {
			t.Fatal("dry-run must not step")
			return nil
		},
	}
	got := handleRequestWithPolicy(context.Background(), request{
		V: 1, ID: "time-dry", Method: "time_hint",
		Params: json.RawMessage(`{"host_unix_ms":1785387600000,"reason":"manual"}`),
	}, policy)
	if !got.OK || got.Result.(map[string]any)["action"] != "skipped" {
		t.Fatalf("response = %#v", got)
	}

	invalid := handleRequestWithPolicy(context.Background(), request{
		V: 1, ID: "time-invalid", Method: "time_hint",
		Params: json.RawMessage(`{"host_unix_ms":1785387600000,"reason":"resume"}`),
	}, policy)
	if invalid.OK || invalid.Error == nil || invalid.Error.Code != "proto" {
		t.Fatalf("response = %#v", invalid)
	}
}

func TestExecFailureIncludesExitAndSeparatedOutput(t *testing.T) {
	req := request{
		V: 1, ID: "exec-fail", Method: "exec",
		Params: json.RawMessage(`{"cmd":["/bin/sh","-c","printf out; printf err >&2; exit 7"]}`),
	}
	got := handleRequest(context.Background(), req)
	if got.OK || got.Error == nil || got.Error.Code != "exec_failed" {
		t.Fatalf("response = %#v", got)
	}
	if got.Error.Details["exit"] != 7 {
		t.Fatalf("exit = %#v", got.Error.Details["exit"])
	}
	if got.Error.Details["stdout"] != "out" || got.Error.Details["stderr"] != "err" {
		t.Fatalf("details = %#v", got.Error.Details)
	}
}

func TestExecTimeoutAndCaps(t *testing.T) {
	req := request{
		V: 1, ID: "exec-timeout", Method: "exec",
		Params: json.RawMessage(`{"cmd":["/bin/sh","-c","sleep 2"],"timeout_ms":25}`),
	}
	started := time.Now()
	got := handleRequest(context.Background(), req)
	if got.OK || got.Error == nil || got.Error.Code != "timeout" {
		t.Fatalf("response = %#v", got)
	}
	if time.Since(started) > time.Second {
		t.Fatal("exec timeout did not stop the process promptly")
	}

	oversized := execParams{Cmd: []string{"/bin/true"}, StdinB64: pointer(base64.StdEncoding.EncodeToString(make([]byte, maxExecStdin+1)))}
	if _, _, err := validateExecParams(oversized); err == nil {
		t.Fatal("expected oversized stdin to fail")
	}
}

func TestExecOutputIsTruncatedWhileProcessCompletes(t *testing.T) {
	req := request{
		V: 1, ID: "exec-truncate", Method: "exec",
		Params: json.RawMessage(`{"cmd":["/bin/sh","-c","i=0; while [ $i -lt 300000 ]; do printf x; i=$((i+1)); done"]}`),
	}
	got := handleRequest(context.Background(), req)
	if !got.OK {
		t.Fatalf("response = %#v", got)
	}
	result := got.Result.(map[string]any)
	if result["truncated"] != true || len(result["stdout"].(string)) != maxExecStream {
		t.Fatalf("result = truncated:%#v stdout-len:%d", result["truncated"], len(result["stdout"].(string)))
	}
}

func TestGuestAddressRejectsLoopbackAndDotZero(t *testing.T) {
	for _, value := range []string{"127.0.0.1", "10.90.1.0", "::1"} {
		if isGuestAddress(net.ParseIP(value)) {
			t.Fatalf("%s must not be accepted", value)
		}
	}
	for _, value := range []string{"10.90.1.10", "fe80::10"} {
		if !isGuestAddress(net.ParseIP(value)) {
			t.Fatalf("%s must be accepted", value)
		}
	}
}

func TestCancelStopsInflightExec(t *testing.T) {
	token := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	client, agent := net.Pipe()
	defer client.Close()
	go (&server{token: []byte(token)}).serveConn(agent)

	assertOK(t, exchange(t, client, map[string]any{
		"v": 1, "id": "hello-1", "method": "hello",
		"params": map[string]any{"token": token, "helper_version": "test"},
	}))
	writeRequest(t, client, map[string]any{
		"v": 1, "id": "exec-slow", "method": "exec",
		"params": map[string]any{"cmd": []string{"/bin/sh", "-c", "sleep 5"}},
	})
	writeRequest(t, client, map[string]any{
		"v": 1, "id": "cancel-1", "method": "cancel",
		"params": map[string]any{"id": "exec-slow"},
	})

	responses := []map[string]any{readResponse(t, client), readResponse(t, client)}
	byID := map[string]map[string]any{}
	for _, response := range responses {
		byID[response["id"].(string)] = response
	}
	assertOK(t, byID["cancel-1"])
	if byID["cancel-1"]["result"].(map[string]any)["cancelled"] != true {
		t.Fatalf("cancel response = %#v", byID["cancel-1"])
	}
	if byID["exec-slow"]["error"].(map[string]any)["code"] != "timeout" {
		t.Fatalf("exec response = %#v", byID["exec-slow"])
	}
}

func TestAuthenticationFailureClosesConnection(t *testing.T) {
	token := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	client, agent := net.Pipe()
	defer client.Close()
	done := make(chan error, 1)
	go func() {
		done <- (&server{token: []byte(token)}).serveConn(agent)
	}()

	got := exchange(t, client, map[string]any{
		"v": 1, "id": "hello-1", "method": "hello",
		"params": map[string]any{"token": "wrong", "helper_version": "test"},
	})
	if got["error"].(map[string]any)["code"] != "auth" {
		t.Fatalf("auth response = %#v", got)
	}
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("server did not close after auth failure")
	}
}

func TestFrameLimit(t *testing.T) {
	var prefix [4]byte
	binary.LittleEndian.PutUint32(prefix[:], maxFrameSize+1)
	if _, err := readFrame(&prefixReader{data: prefix[:]}); err == nil {
		t.Fatal("expected oversized frame to fail")
	}
}

type prefixReader struct {
	data []byte
}

func (r *prefixReader) Read(p []byte) (int, error) {
	n := copy(p, r.data)
	r.data = r.data[n:]
	return n, nil
}

func exchange(t *testing.T, conn net.Conn, req map[string]any) map[string]any {
	t.Helper()
	writeRequest(t, conn, req)
	return readResponse(t, conn)
}

func writeRequest(t *testing.T, conn net.Conn, req map[string]any) {
	t.Helper()
	payload, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	var prefix [4]byte
	binary.LittleEndian.PutUint32(prefix[:], uint32(len(payload)))
	if _, err := conn.Write(append(prefix[:], payload...)); err != nil {
		t.Fatal(err)
	}
}

func readResponse(t *testing.T, conn net.Conn) map[string]any {
	t.Helper()
	responsePayload, err := readFrame(conn)
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(responsePayload, &got); err != nil {
		t.Fatal(err)
	}
	return got
}

func pointer(value string) *string {
	return &value
}

func assertOK(t *testing.T, got map[string]any) {
	t.Helper()
	if ok, _ := got["ok"].(bool); !ok {
		t.Fatalf("response = %#v", got)
	}
}
