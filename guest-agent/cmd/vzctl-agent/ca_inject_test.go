package main

import (
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"math/big"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestCAInjectRunsPrivilegedInstaller(t *testing.T) {
	certificate, fingerprint := testCA()
	previous := runCAInstaller
	defer func() { runCAInstaller = previous }()
	var gotPEM, gotName, gotFingerprint string
	runCAInstaller = func(pem, name, fingerprint string) error {
		gotPEM, gotName, gotFingerprint = pem, name, fingerprint
		return nil
	}

	request := request{
		V:      1,
		ID:     "ca-1",
		Method: "ca_inject",
		Params: mustJSON(t, map[string]any{
			"pem": certificate, "fingerprint": fingerprint, "name": "vzctl-local",
		}),
	}
	response := handleCAInject(request)
	if !response.OK {
		t.Fatalf("response = %#v", response)
	}
	if gotPEM != certificate || gotName != "vzctl-local" || gotFingerprint != fingerprint {
		t.Fatalf("installer args = %q %q %q", gotPEM, gotName, gotFingerprint)
	}
}

func TestCAInjectRejectsFingerprintMismatchAndInvalidName(t *testing.T) {
	certificate, fingerprint := testCA()
	previous := runCAInstaller
	defer func() { runCAInstaller = previous }()
	called := false
	runCAInstaller = func(_, _, _ string) error {
		called = true
		return nil
	}

	for _, params := range []map[string]any{
		{"pem": certificate, "fingerprint": strings.Repeat("0", 64)},
		{"pem": certificate, "fingerprint": fingerprint, "name": "../bad"},
	} {
		response := handleCAInject(request{V: 1, ID: "ca-invalid", Params: mustJSON(t, params)})
		if response.OK || response.Error == nil || response.Error.Code != "proto" {
			t.Fatalf("response = %#v", response)
		}
	}
	if called {
		t.Fatal("installer must not run for invalid input")
	}
}

func TestCAInjectPropagatesInstallerFailure(t *testing.T) {
	certificate, fingerprint := testCA()
	previous := runCAInstaller
	defer func() { runCAInstaller = previous }()
	runCAInstaller = func(_, _, _ string) error { return errors.New("trust update failed") }

	response := handleCAInject(request{
		V: 1, ID: "ca-fail", Params: mustJSON(t, map[string]any{
			"pem": certificate, "fingerprint": fingerprint,
		}),
	})
	if response.OK || response.Error == nil || !strings.Contains(response.Error.Message, "trust update failed") {
		t.Fatalf("response = %#v", response)
	}
}

func TestCAInjectScriptInstallsAndVerifies(t *testing.T) {
	certificate, fingerprint := validTestCA(t)
	directory := t.TempDir()
	caDirectory := filepath.Join(directory, "ca")
	fingerprintFile := filepath.Join(directory, "state", "ca.fingerprint")
	trustBundle := filepath.Join(directory, "ca-certificates.crt")
	updateScript := filepath.Join(directory, "update-ca-certificates")
	updateBody := "#!/bin/sh\ncp \"$VZCTL_CA_CERT_DIR/vzctl-local.crt\" \"$VZCTL_CA_TRUST_BUNDLE\"\n"
	if err := os.WriteFile(updateScript, []byte(updateBody), 0o755); err != nil {
		t.Fatal(err)
	}

	command := exec.Command("sh", filepath.Join("..", "..", "scripts", "ca-inject"), "vzctl-local", fingerprint)
	command.Stdin = strings.NewReader(certificate)
	command.Env = append(os.Environ(),
		"VZCTL_CA_CERT_DIR="+caDirectory,
		"VZCTL_CA_FINGERPRINT_FILE="+fingerprintFile,
		"VZCTL_CA_TRUST_BUNDLE="+trustBundle,
		"VZCTL_UPDATE_CA_BIN="+updateScript,
	)
	if output, err := command.CombinedOutput(); err != nil {
		t.Fatalf("ca-inject: %v: %s", err, output)
	}
	installed, err := os.ReadFile(filepath.Join(caDirectory, "vzctl-local.crt"))
	if err != nil || string(installed) != certificate {
		t.Fatalf("installed CA mismatch: %v", err)
	}
	stored, err := os.ReadFile(fingerprintFile)
	if err != nil || strings.TrimSpace(string(stored)) != fingerprint {
		t.Fatalf("stored fingerprint mismatch: %v", err)
	}
}

func testCA() (string, string) {
	der := []byte("vzctl test CA DER")
	block := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	sum := sha256.Sum256(der)
	return string(block), hex.EncodeToString(sum[:])
}

func validTestCA(t *testing.T) (string, string) {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "vzctl test CA"},
		NotBefore:             time.Now().Add(-time.Minute),
		NotAfter:              time.Now().Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	encoded := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	sum := sha256.Sum256(der)
	return string(encoded), hex.EncodeToString(sum[:])
}

func mustJSON(t *testing.T, value any) json.RawMessage {
	t.Helper()
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}
