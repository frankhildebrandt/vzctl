package main

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPublishHTTPPutListDelete(t *testing.T) {
	socket := filepath.Join("/tmp", "vzctl-pub-http.sock")
	_ = os.Remove(socket)
	registry := newServiceRegistry()
	registry.alive = func(int) bool { return true }
	server, err := startPublishHTTP(registry, socket)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = server.Close()
		_ = os.Remove(socket)
	})

	client := unixHTTPClient(socket)
	put, err := http.NewRequest(http.MethodPut, "http://localhost/v1/services/app", strings.NewReader(`{"kind":"iwatch","url":"http://127.0.0.1:8787","pid":1}`))
	if err != nil {
		t.Fatal(err)
	}
	resp, err := client.Do(put)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusNoContent {
		t.Fatalf("put status = %d", resp.StatusCode)
	}

	list, err := client.Get("http://localhost/v1/services")
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(list.Body)
	_ = list.Body.Close()
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		t.Fatal(err)
	}
	services, _ := payload["services"].([]any)
	if len(services) != 1 {
		t.Fatalf("services = %#v", payload)
	}

	del, err := http.NewRequest(http.MethodDelete, "http://localhost/v1/services/app", nil)
	if err != nil {
		t.Fatal(err)
	}
	resp, err = client.Do(del)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	if _, ok := registry.get("app"); ok {
		t.Fatal("expected delete")
	}
}

func TestPublishHTTPRejectsBadURL(t *testing.T) {
	socket := filepath.Join("/tmp", "vzctl-pub-http-bad.sock")
	_ = os.Remove(socket)
	server, err := startPublishHTTP(newServiceRegistry(), socket)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = server.Close()
		_ = os.Remove(socket)
	})
	client := unixHTTPClient(socket)
	put, err := http.NewRequest(http.MethodPut, "http://localhost/v1/services/app", strings.NewReader(`{"kind":"iwatch","url":"http://10.1.1.1:80"}`))
	if err != nil {
		t.Fatal(err)
	}
	resp, err := client.Do(put)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("status = %d", resp.StatusCode)
	}
}
