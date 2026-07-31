//go:build linux

package main

import (
	"fmt"
	"os"
	"syscall"
	"unsafe"
)

func openPTY() (ptm *os.File, pts *os.File, err error) {
	ptm, err = os.OpenFile("/dev/ptmx", os.O_RDWR|syscall.O_CLOEXEC, 0)
	if err != nil {
		return nil, nil, err
	}
	if err := unlockpt(ptm); err != nil {
		_ = ptm.Close()
		return nil, nil, err
	}
	name, err := ptsname(ptm)
	if err != nil {
		_ = ptm.Close()
		return nil, nil, err
	}
	pts, err = os.OpenFile(name, os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		_ = ptm.Close()
		return nil, nil, err
	}
	return ptm, pts, nil
}

func setPTYSize(ptm *os.File, cols, rows uint16) error {
	if cols == 0 {
		cols = 80
	}
	if rows == 0 {
		rows = 24
	}
	var size struct {
		Row    uint16
		Col    uint16
		Xpixel uint16
		Ypixel uint16
	}
	size.Row = rows
	size.Col = cols
	_, _, errno := syscall.Syscall(
		syscall.SYS_IOCTL,
		ptm.Fd(),
		uintptr(syscall.TIOCSWINSZ),
		uintptr(unsafe.Pointer(&size)),
	)
	if errno != 0 {
		return errno
	}
	return nil
}

func unlockpt(f *os.File) error {
	var unlock int32
	_, _, errno := syscall.Syscall(
		syscall.SYS_IOCTL,
		f.Fd(),
		uintptr(syscall.TIOCSPTLCK),
		uintptr(unsafe.Pointer(&unlock)),
	)
	if errno != 0 {
		return errno
	}
	return nil
}

func ptsname(f *os.File) (string, error) {
	var n uint32
	_, _, errno := syscall.Syscall(
		syscall.SYS_IOCTL,
		f.Fd(),
		uintptr(syscall.TIOCGPTN),
		uintptr(unsafe.Pointer(&n)),
	)
	if errno != 0 {
		return "", errno
	}
	return fmt.Sprintf("/dev/pts/%d", n), nil
}
