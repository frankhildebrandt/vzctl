package main

import (
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

func TestHelloPingVersionHealthAndUnsupported(t *testing.T) {
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
	if len(caps) != 3 {
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

	unsupported := exchange(t, client, map[string]any{
		"v": 1, "id": "exec-1", "method": "exec", "params": map[string]any{},
	})
	if unsupported["error"].(map[string]any)["code"] != "unsupported" {
		t.Fatalf("unsupported response = %#v", unsupported)
	}

	_ = client.Close()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("server did not stop")
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
	payload, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	var prefix [4]byte
	binary.LittleEndian.PutUint32(prefix[:], uint32(len(payload)))
	if _, err := conn.Write(append(prefix[:], payload...)); err != nil {
		t.Fatal(err)
	}
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

func assertOK(t *testing.T, got map[string]any) {
	t.Helper()
	if ok, _ := got["ok"].(bool); !ok {
		t.Fatalf("response = %#v", got)
	}
}
