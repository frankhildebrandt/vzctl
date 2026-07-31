//go:build !linux

package main

import (
	"errors"
	"os"
)

func openPTY() (*os.File, *os.File, error) {
	return nil, nil, errors.New("PTY exec requires Linux")
}

func setPTYSize(ptm *os.File, cols, rows uint16) error {
	return errors.New("PTY exec requires Linux")
}
