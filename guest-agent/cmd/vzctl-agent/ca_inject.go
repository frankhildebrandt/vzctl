package main

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"path"
	"strings"
)

const (
	caCertDir      = "/usr/local/share/ca-certificates"
	caFingerprintF = "/var/lib/vzctl/ca.fingerprint"
	updateCABin    = "/usr/sbin/update-ca-certificates"
)

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

	dest := path.Join(caCertDir, name+".crt")
	tmp := dest + ".tmp"
	if err := os.MkdirAll(caCertDir, 0o755); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	if err := os.WriteFile(tmp, []byte(params.PEM), 0o644); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	if err := os.Rename(tmp, dest); err != nil {
		_ = os.Remove(tmp)
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	if err := os.MkdirAll("/var/lib/vzctl", 0o755); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	if err := os.WriteFile(caFingerprintF, []byte(want+"\n"), 0o644); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}

	cmd := exec.Command("sudo", "-n", updateCABin)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return errorResponse(req.ID, "internal", fmt.Sprintf("update-ca-certificates: %v: %s", err, strings.TrimSpace(string(out))), map[string]any{
			"name": name,
		})
	}
	return successResponse(req.ID, map[string]any{
		"installed":   true,
		"fingerprint": got,
		"name":        name,
		"path":        dest,
	})
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
