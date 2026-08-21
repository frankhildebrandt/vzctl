package main

import "testing"

func TestValidateServiceName(t *testing.T) {
	tests := []struct {
		name    string
		input   string
		wantErr bool
	}{
		{name: "ok", input: "app"},
		{name: "hyphen", input: "my-app"},
		{name: "empty", input: "", wantErr: true},
		{name: "svc", input: "svc", wantErr: true},
		{name: "underscore", input: "my_app", wantErr: true},
		{name: "leading hyphen", input: "-app", wantErr: true},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			err := validateServiceName(test.input)
			if test.wantErr && err == nil {
				t.Fatal("expected error")
			}
			if !test.wantErr && err != nil {
				t.Fatal(err)
			}
		})
	}
}
