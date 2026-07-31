package main

import (
	"encoding/binary"
	"errors"
	"io"
)

const (
	muxStdin    byte = 0x01
	muxStdout   byte = 0x02
	muxResize   byte = 0x04
	muxExit     byte = 0x05
	muxStdinEOF byte = 0x06
	maxMuxFrame      = maxFrameSize
)

func writeMuxFrame(w io.Writer, frameType byte, payload []byte) error {
	if len(payload) > maxMuxFrame {
		return errors.New("mux frame exceeds 1 MiB")
	}
	header := [5]byte{frameType}
	binary.LittleEndian.PutUint32(header[1:], uint32(len(payload)))
	if _, err := w.Write(header[:]); err != nil {
		return err
	}
	if len(payload) == 0 {
		return nil
	}
	_, err := w.Write(payload)
	return err
}

func readMuxFrame(r io.Reader) (byte, []byte, error) {
	var header [5]byte
	if _, err := io.ReadFull(r, header[:]); err != nil {
		return 0, nil, err
	}
	length := binary.LittleEndian.Uint32(header[1:])
	if length == 0 {
		return header[0], nil, nil
	}
	if length > maxMuxFrame {
		return 0, nil, errors.New("mux frame exceeds 1 MiB")
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(r, payload); err != nil {
		return 0, nil, err
	}
	return header[0], payload, nil
}
