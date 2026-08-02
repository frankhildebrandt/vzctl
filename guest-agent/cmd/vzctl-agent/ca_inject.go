package main

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

const (
	caInjectBin = "/usr/local/lib/vzctl/ca-inject"
)

var runCAInstaller = executeCAInstaller

type caInjectParams struct {
	PEM         string `json:"pem"`
	Fingerprint string `json:"fingerprint"`
	Name        string `json:"name"`
}

func handleCAInject(req request) response {
	var params caInjectParams
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid ca_inject parameters", nil)
	}
	if strings.TrimSpace(params.PEM) == "" {
		return errorResponse(req.ID, "proto", "ca_inject requires pem", nil)
	}
	if strings.TrimSpace(params.Fingerprint) == "" {
		return errorResponse(req.ID, "proto", "ca_inject requires fingerprint", nil)
	}
	name := params.Name
	if name == "" {
		name = "vzctl-local"
	}
	if strings.ContainsAny(name, "/.\\") || len(name) > 64 {
		return errorResponse(req.ID, "proto", "invalid ca_inject name", nil)
	}

	got := fingerprintPEM(params.PEM)
	want := strings.TrimPrefix(strings.ToLower(strings.TrimSpace(params.Fingerprint)), "sha256:")
	if got != want {
		return errorResponse(req.ID, "proto", "ca_inject fingerprint mismatch", map[string]any{
			"expected": want,
			"got":      got,
		})
	}

	if err := runCAInstaller(params.PEM, name, want); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), map[string]any{
			"name": name,
		})
	}
	return successResponse(req.ID, map[string]any{
		"installed":   true,
		"fingerprint": got,
		"name":        name,
		"path":        "/usr/local/share/ca-certificates/" + name + ".crt",
	})
}

func executeCAInstaller(pem, name, fingerprint string) error {
	binary := caInjectBin
	if override := strings.TrimSpace(os.Getenv("VZCTL_CA_INJECT")); override != "" {
		binary = override
	}
	cmd := exec.Command("sudo", "-n", binary, name, fingerprint)
	cmd.Stdin = strings.NewReader(pem)
	out, err := cmd.CombinedOutput()
	if err == nil {
		return nil
	}
	detail := strings.TrimSpace(string(out))
	if detail == "" {
		detail = err.Error()
	}
	return fmt.Errorf("ca-inject failed: %s", detail)
}

func fingerprintPEM(pem string) string {
	if i := strings.Index(pem, "-----BEGIN"); i >= 0 {
		rest := pem[i:]
		if j := strings.Index(rest, "-----END"); j >= 0 {
			block := rest[:j]
			var b64 strings.Builder
			for _, line := range strings.Split(block, "\n") {
				line = strings.TrimSpace(line)
				if line == "" || strings.HasPrefix(line, "-----") {
					continue
				}
				b64.WriteString(line)
			}
			if decoded, err := base64.StdEncoding.DecodeString(b64.String()); err == nil {
				sum := sha256.Sum256(decoded)
				return hex.EncodeToString(sum[:])
			}
		}
	}
	sum := sha256.Sum256([]byte(pem))
	return hex.EncodeToString(sum[:])
}
