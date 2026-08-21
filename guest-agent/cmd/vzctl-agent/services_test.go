package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func TestHandleServicesListAndHTTP(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/status" {
			t.Fatalf("path = %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	t.Cleanup(upstream.Close)

	parsed, _ := url.Parse(upstream.URL)
	origin := "http://127.0.0.1:" + parsed.Port()
	registry := newServiceRegistry()
	registry.alive = func(int) bool { return true }
	if err := registry.put(publishedService{Name: "app", Kind: "iwatch", URL: origin, PID: 1}); err != nil {
		t.Fatal(err)
	}

	listed := handleServicesList(request{V: 1, ID: "list", Method: "services.list", Params: json.RawMessage(`{}`)}, registry)
	if !listed.OK {
		t.Fatalf("list = %#v", listed)
	}

	got := handleServicesHTTP(context.Background(), request{
		V: 1, ID: "http", Method: "services.http",
		Params: json.RawMessage(`{"name":"app","method":"GET","path":"/api/status"}`),
	}, registry)
	if !got.OK {
		t.Fatalf("http = %#v", got)
	}
	result := got.Result.(map[string]any)
	if result["status"] != 200 {
		t.Fatalf("status = %#v", result["status"])
	}
	body, err := base64.StdEncoding.DecodeString(result["body_b64"].(string))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), `"ok":true`) {
		t.Fatalf("body = %s", body)
	}
}

func TestHandleServicesHTTPRestart(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/api/restart" {
			t.Fatalf("got %s %s", r.Method, r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":"restarted"}`))
	}))
	t.Cleanup(upstream.Close)

	parsed, _ := url.Parse(upstream.URL)
	origin := "http://127.0.0.1:" + parsed.Port()
	registry := newServiceRegistry()
	registry.alive = func(int) bool { return true }
	if err := registry.put(publishedService{Name: "app", Kind: "iwatch", URL: origin, PID: 1}); err != nil {
		t.Fatal(err)
	}

	got := handleServicesHTTP(context.Background(), request{
		V: 1, ID: "http", Method: "services.http",
		Params: json.RawMessage(`{"name":"app","method":"POST","path":"/api/restart"}`),
	}, registry)
	if !got.OK {
		t.Fatalf("http = %#v", got)
	}
	result := got.Result.(map[string]any)
	if result["status"] != 200 {
		t.Fatalf("status = %#v", result["status"])
	}
}

func TestHandleServicesHTTPUnknownName(t *testing.T) {
	got := handleServicesHTTP(context.Background(), request{
		V: 1, ID: "http", Method: "services.http",
		Params: json.RawMessage(`{"name":"missing","path":"/api/status"}`),
	}, newServiceRegistry())
	if got.OK || got.Error == nil || got.Error.Code != "not_found" {
		t.Fatalf("got = %#v", got)
	}
}

func TestValidateServicePath(t *testing.T) {
	if err := validateServicePath("/api/logs?q=a"); err != nil {
		t.Fatal(err)
	}
	if err := validateServicePath("http://evil/api"); err == nil {
		t.Fatal("expected host path to fail")
	}
	if err := validateServicePath("//evil/api"); err == nil {
		t.Fatal("expected scheme-relative path to fail")
	}
}

func TestPrepareServiceStream(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte("event: line\ndata: {}\n\n"))
	}))
	t.Cleanup(upstream.Close)
	parsed, _ := url.Parse(upstream.URL)
	origin := "http://127.0.0.1:" + parsed.Port()
	registry := newServiceRegistry()
	registry.alive = func(int) bool { return true }
	if err := registry.put(publishedService{Name: "app", Kind: "iwatch", URL: origin, PID: 1}); err != nil {
		t.Fatal(err)
	}
	respJSON, httpResp := prepareServiceStream(context.Background(), request{
		V: 1, ID: "stream", Method: "services.stream",
		Params: json.RawMessage(`{"name":"app","path":"/api/logs/sse"}`),
	}, registry)
	if httpResp != nil {
		t.Cleanup(func() { _ = httpResp.Body.Close() })
	}
	if !respJSON.OK {
		t.Fatalf("prepare = %#v", respJSON)
	}
	result := respJSON.Result.(map[string]any)
	if result["upgraded"] != true {
		t.Fatalf("result = %#v", result)
	}
}
