package main

import (
	"errors"
	"os"
	"sync"
	"syscall"
)

var errKindRequired = errors.New("kind is required")

type serviceRegistry struct {
	mu    sync.Mutex
	items map[string]publishedService
	alive func(pid int) bool
}

func newServiceRegistry() *serviceRegistry {
	return &serviceRegistry{
		items: map[string]publishedService{},
		alive: processAlive,
	}
}

func (r *serviceRegistry) put(service publishedService) error {
	if err := validateServiceName(service.Name); err != nil {
		return err
	}
	if service.Kind == "" {
		return errKindRequired
	}
	if err := validateLoopbackURL(service.URL); err != nil {
		return err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	r.items[service.Name] = service
	return nil
}

func (r *serviceRegistry) delete(name string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.items, name)
}

func (r *serviceRegistry) get(name string) (publishedService, bool) {
	r.reap()
	r.mu.Lock()
	defer r.mu.Unlock()
	service, ok := r.items[name]
	return service, ok
}

func (r *serviceRegistry) list() []publishedService {
	r.reap()
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]publishedService, 0, len(r.items))
	for _, service := range r.items {
		out = append(out, service)
	}
	return out
}

func (r *serviceRegistry) reap() {
	if r == nil {
		return
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.alive == nil {
		return
	}
	for name, service := range r.items {
		if service.PID > 0 && !r.alive(service.PID) {
			delete(r.items, name)
		}
	}
}

func processAlive(pid int) bool {
	process, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	return process.Signal(syscall.Signal(0)) == nil
}
