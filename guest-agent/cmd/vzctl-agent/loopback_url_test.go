package main

import "testing"

func TestValidateLoopbackURL(t *testing.T) {
	tests := []struct {
		name    string
		input   string
		wantErr bool
	}{
		{name: "ipv4", input: "http://127.0.0.1:8787"},
		{name: "localhost", input: "http://localhost:8787"},
		{name: "ipv6", input: "http://[::1]:8787"},
		{name: "lan", input: "http://10.0.0.10:80", wantErr: true},
		{name: "no port", input: "http://127.0.0.1", wantErr: true},
		{name: "ftp", input: "ftp://127.0.0.1:21", wantErr: true},
		{name: "empty", input: "", wantErr: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			err := validateLoopbackURL(test.input)
			if test.wantErr && err == nil {
				t.Fatal("expected error")
			}
			if !test.wantErr && err != nil {
				t.Fatal(err)
			}
		})
	}
}
