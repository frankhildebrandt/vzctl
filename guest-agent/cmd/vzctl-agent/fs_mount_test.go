package main

import "testing"

func TestValidateFSMountName(t *testing.T) {
	if err := validateFSMountName("web-src"); err != nil {
		t.Fatalf("expected valid name: %v", err)
	}
	if err := validateFSMountName("vzctl"); err == nil {
		t.Fatal("reserved tag must be rejected")
	}
	if err := validateFSMountName("bad.name"); err == nil {
		t.Fatal("dot must be rejected")
	}
}

func TestValidateFSMountTarget(t *testing.T) {
	if err := validateFSMountTarget("/srv/app"); err != nil {
		t.Fatalf("expected valid target: %v", err)
	}
	if err := validateFSMountTarget("/"); err == nil {
		t.Fatal("root target must be rejected")
	}
	if err := validateFSMountTarget("relative"); err == nil {
		t.Fatal("relative target must be rejected")
	}
	if err := validateFSMountTarget("/srv/../etc"); err == nil {
		t.Fatal("unclean target must be rejected")
	}
}
