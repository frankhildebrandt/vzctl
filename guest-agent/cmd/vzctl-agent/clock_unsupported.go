//go:build !linux

package main

import (
	"errors"
	"time"
)

func setSystemClock(time.Time) error {
	return errors.New("clock stepping is supported only on Linux")
}
