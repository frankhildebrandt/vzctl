//go:build !linux

package main

import (
	"errors"
	"net"
)

func listenVsock(_ uint32) (net.Listener, error) {
	return nil, errors.New("virtio-vsock listener is supported only on Linux")
}
