package main

import (
	"bytes"
	"testing"
)

func TestMuxFrameRoundTrip(t *testing.T) {
	var buffer bytes.Buffer
	payload := []byte("hello-pty")
	if err := writeMuxFrame(&buffer, muxStdout, payload); err != nil {
		t.Fatal(err)
	}
	frameType, got, err := readMuxFrame(&buffer)
	if err != nil {
		t.Fatal(err)
	}
	if frameType != muxStdout {
		t.Fatalf("type = %d", frameType)
	}
	if string(got) != string(payload) {
		t.Fatalf("payload = %q", got)
	}
}

func TestMuxEmptyFrame(t *testing.T) {
	var buffer bytes.Buffer
	if err := writeMuxFrame(&buffer, muxStdinEOF, nil); err != nil {
		t.Fatal(err)
	}
	frameType, got, err := readMuxFrame(&buffer)
	if err != nil {
		t.Fatal(err)
	}
	if frameType != muxStdinEOF || len(got) != 0 {
		t.Fatalf("got type=%d payload=%q", frameType, got)
	}
}

func TestCapabilitiesIncludeExecTTY(t *testing.T) {
	found := false
	for _, capability := range capabilities {
		if capability == "exec_tty" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("capabilities = %#v", capabilities)
	}
}
