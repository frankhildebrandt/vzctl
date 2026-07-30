//go:build linux

package main

import (
	"net"
	"os"
	"syscall"
	"time"
	"unsafe"
)

const (
	afVsock      = 40
	vmaddrCIDAny = ^uint32(0)
)

type rawSockaddrVM struct {
	Family    uint16
	Reserved1 uint16
	Port      uint32
	CID       uint32
	Flags     uint8
	Zero      [3]byte
}

type vsockAddr struct {
	port uint32
}

func (a vsockAddr) Network() string { return "vsock" }
func (a vsockAddr) String() string  { return "any:" + itoa(a.port) }

type vsockListener struct {
	file *os.File
	addr vsockAddr
}

func (l *vsockListener) Accept() (net.Conn, error) {
	fd, _, errno := syscall.Syscall6(
		syscall.SYS_ACCEPT4,
		l.file.Fd(),
		0,
		0,
		syscall.SOCK_CLOEXEC,
		0,
		0,
	)
	if errno != 0 {
		return nil, os.NewSyscallError("accept4", errno)
	}
	file := os.NewFile(fd, "vsock-connection")
	if file == nil {
		_ = syscall.Close(int(fd))
		return nil, syscall.EBADF
	}
	return &vsockConn{
		file:   file,
		local:  l.addr,
		remote: vsockAddr{},
	}, nil
}

func (l *vsockListener) Close() error   { return l.file.Close() }
func (l *vsockListener) Addr() net.Addr { return l.addr }

type vsockConn struct {
	file   *os.File
	local  vsockAddr
	remote vsockAddr
}

func (c *vsockConn) Read(p []byte) (int, error)         { return c.file.Read(p) }
func (c *vsockConn) Write(p []byte) (int, error)        { return c.file.Write(p) }
func (c *vsockConn) Close() error                       { return c.file.Close() }
func (c *vsockConn) LocalAddr() net.Addr                { return c.local }
func (c *vsockConn) RemoteAddr() net.Addr               { return c.remote }
func (c *vsockConn) SetDeadline(t time.Time) error      { return c.file.SetDeadline(t) }
func (c *vsockConn) SetReadDeadline(t time.Time) error  { return c.file.SetReadDeadline(t) }
func (c *vsockConn) SetWriteDeadline(t time.Time) error { return c.file.SetWriteDeadline(t) }

func listenVsock(port uint32) (net.Listener, error) {
	fd, err := syscall.Socket(afVsock, syscall.SOCK_STREAM|syscall.SOCK_CLOEXEC, 0)
	if err != nil {
		return nil, err
	}
	closeFD := true
	defer func() {
		if closeFD {
			_ = syscall.Close(fd)
		}
	}()

	address := rawSockaddrVM{
		Family: afVsock,
		Port:   port,
		CID:    vmaddrCIDAny,
	}
	_, _, errno := syscall.Syscall(
		syscall.SYS_BIND,
		uintptr(fd),
		uintptr(unsafe.Pointer(&address)),
		unsafe.Sizeof(address),
	)
	if errno != 0 {
		return nil, errno
	}
	if err := syscall.Listen(fd, syscall.SOMAXCONN); err != nil {
		return nil, err
	}

	file := os.NewFile(uintptr(fd), "vsock-listener")
	if file == nil {
		return nil, syscall.EBADF
	}
	closeFD = false
	return &vsockListener{file: file, addr: vsockAddr{port: port}}, nil
}

func itoa(value uint32) string {
	if value == 0 {
		return "0"
	}
	var buffer [10]byte
	index := len(buffer)
	for value > 0 {
		index--
		buffer[index] = byte('0' + value%10)
		value /= 10
	}
	return string(buffer[index:])
}
