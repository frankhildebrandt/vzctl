//go:build linux

package main

import (
	"syscall"
	"time"
	"unsafe"
)

func setSystemClock(value time.Time) error {
	spec := syscall.NsecToTimespec(value.UnixNano())
	_, _, errno := syscall.RawSyscall(
		syscall.SYS_CLOCK_SETTIME,
		uintptr(0),
		uintptr(unsafe.Pointer(&spec)),
		0,
	)
	if errno != 0 {
		return errno
	}
	return nil
}
