package main

import (
	"os"
	"testing"
)

func TestServiceRegistryPutListDelete(t *testing.T) {
	registry := newServiceRegistry()
	registry.alive = func(int) bool { return true }

	err := registry.put(publishedService{
		Name: "app",
		Kind: "iwatch",
		URL:  "http://127.0.0.1:8787",
		PID:  os.Getpid(),
	})
	if err != nil {
		t.Fatal(err)
	}
	got, ok := registry.get("app")
	if !ok || got.Kind != "iwatch" {
		t.Fatalf("get = %+v ok=%v", got, ok)
	}
	if len(registry.list()) != 1 {
		t.Fatalf("list len = %d", len(registry.list()))
	}
	registry.delete("app")
	if _, ok := registry.get("app"); ok {
		t.Fatal("expected delete")
	}
}

func TestServiceRegistryRejectsLAN(t *testing.T) {
	registry := newServiceRegistry()
	err := registry.put(publishedService{
		Name: "app",
		Kind: "iwatch",
		URL:  "http://10.0.0.8:80",
	})
	if err == nil {
		t.Fatal("expected lan url to fail")
	}
}

func TestServiceRegistryReapsDeadPID(t *testing.T) {
	registry := newServiceRegistry()
	registry.alive = func(pid int) bool { return pid != 4242 }
	if err := registry.put(publishedService{
		Name: "app",
		Kind: "iwatch",
		URL:  "http://127.0.0.1:8787",
		PID:  4242,
	}); err != nil {
		t.Fatal(err)
	}
	if _, ok := registry.get("app"); ok {
		t.Fatal("dead pid should be reaped")
	}
}

func TestServiceRegistryKeepsPIDZero(t *testing.T) {
	registry := newServiceRegistry()
	registry.alive = func(int) bool { return false }
	if err := registry.put(publishedService{
		Name: "app",
		Kind: "iwatch",
		URL:  "http://127.0.0.1:1",
		PID:  0,
	}); err != nil {
		t.Fatal(err)
	}
	if _, ok := registry.get("app"); !ok {
		t.Fatal("pid 0 should not reap")
	}
}

func TestServiceRegistryRequiresKind(t *testing.T) {
	registry := newServiceRegistry()
	err := registry.put(publishedService{Name: "app", URL: "http://127.0.0.1:1"})
	if err != errKindRequired {
		t.Fatalf("err = %v", err)
	}
}
