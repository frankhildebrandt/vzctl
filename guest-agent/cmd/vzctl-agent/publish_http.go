package main

import (
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"strings"
)

const guestPublishSocket = "/run/vzctl/guest.sock"

type publishHTTPServer struct {
	registry *serviceRegistry
	listener net.Listener
}

func startPublishHTTP(registry *serviceRegistry, socket string) (*publishHTTPServer, error) {
	if socket == "" {
		socket = guestPublishSocket
	}
	_ = os.Remove(socket)
	listener, err := net.Listen("unix", socket)
	if err != nil {
		return nil, fmt.Errorf("listen publish socket: %w", err)
	}
	if err := os.Chmod(socket, 0o660); err != nil {
		_ = listener.Close()
		return nil, fmt.Errorf("chmod publish socket: %w", err)
	}
	server := &publishHTTPServer{registry: registry, listener: listener}
	mux := http.NewServeMux()
	mux.HandleFunc("PUT /v1/services/{name}", server.handlePut)
	mux.HandleFunc("DELETE /v1/services/{name}", server.handleDelete)
	mux.HandleFunc("GET /v1/services", server.handleList)
	go func() { _ = http.Serve(listener, mux) }()
	return server, nil
}

func (s *publishHTTPServer) Close() error {
	if s == nil || s.listener == nil {
		return nil
	}
	return s.listener.Close()
}

func (s *publishHTTPServer) handlePut(w http.ResponseWriter, r *http.Request) {
	name := r.PathValue("name")
	var body publishedService
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "invalid json", http.StatusBadRequest)
		return
	}
	body.Name = name
	if err := s.registry.put(body); err != nil {
		status := http.StatusBadRequest
		if strings.Contains(err.Error(), "reserved") {
			status = http.StatusConflict
		}
		http.Error(w, err.Error(), status)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *publishHTTPServer) handleDelete(w http.ResponseWriter, r *http.Request) {
	s.registry.delete(r.PathValue("name"))
	w.WriteHeader(http.StatusNoContent)
}

func (s *publishHTTPServer) handleList(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{"services": s.registry.list()})
}
