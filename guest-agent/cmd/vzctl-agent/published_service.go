package main

// publishedService is one named guest HTTP service advertised to the host.
type publishedService struct {
	Name string `json:"name"`
	Kind string `json:"kind"`
	URL  string `json:"url"`
	PID  int    `json:"pid"`
}
