package main

import (
	"bufio"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

type statsCollector struct {
	mu            sync.Mutex
	lastCPU       *cpuSnap
	lastDisk      *diskSnap
	readStat      func() (string, error)
	readMeminfo   func() (string, error)
	readDiskstats func() (string, error)
	readLoadavg   func() (string, error)
	topProcess    func() map[string]any
	now           func() time.Time
}

type cpuSnap struct {
	idle  uint64
	total uint64
	at    time.Time
}

type diskSnap struct {
	reads  uint64
	writes uint64
	at     time.Time
}

func newStatsCollector() *statsCollector {
	return &statsCollector{
		readStat:      func() (string, error) { return readProcFile("/proc/stat") },
		readMeminfo:   func() (string, error) { return readProcFile("/proc/meminfo") },
		readDiskstats: func() (string, error) { return readProcFile("/proc/diskstats") },
		readLoadavg:   func() (string, error) { return readProcFile("/proc/loadavg") },
		topProcess:    sampleTopProcess,
		now:           time.Now,
	}
}

func readProcFile(path string) (string, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	return string(raw), nil
}

func (c *statsCollector) sample() (map[string]any, error) {
	now := c.now()
	cpuText, err := c.readStat()
	if err != nil {
		return nil, err
	}
	memText, err := c.readMeminfo()
	if err != nil {
		return nil, err
	}
	diskText, err := c.readDiskstats()
	if err != nil {
		return nil, err
	}

	idle, total, err := parseCPUStat(cpuText)
	if err != nil {
		return nil, err
	}
	usedMiB, totalMiB, memPercent, err := parseMeminfo(memText)
	if err != nil {
		return nil, err
	}
	reads, writes := parseDiskstats(diskText)

	c.mu.Lock()
	defer c.mu.Unlock()

	var cpuPercent any
	if prev := c.lastCPU; prev != nil {
		dIdle := idle - prev.idle
		dTotal := total - prev.total
		if dTotal > 0 {
			cpuPercent = 100.0 * (1.0 - float64(dIdle)/float64(dTotal))
		}
	}
	c.lastCPU = &cpuSnap{idle: idle, total: total, at: now}

	var readIOPS any
	var writeIOPS any
	if prev := c.lastDisk; prev != nil {
		elapsed := now.Sub(prev.at).Seconds()
		if elapsed <= 0 {
			elapsed = 1
		}
		readIOPS = float64(reads-prev.reads) / elapsed
		writeIOPS = float64(writes-prev.writes) / elapsed
	}
	c.lastDisk = &diskSnap{reads: reads, writes: writes, at: now}

	var load1 any
	if c.readLoadavg != nil {
		if loadText, loadErr := c.readLoadavg(); loadErr == nil {
			load1 = parseLoad1(loadText)
		}
	}
	var top any
	if c.topProcess != nil {
		top = c.topProcess()
	}

	return map[string]any{
		"cpu": map[string]any{"percent": cpuPercent},
		"memory": map[string]any{
			"used_mib":  usedMiB,
			"total_mib": totalMiB,
			"percent":   memPercent,
		},
		"disk": map[string]any{
			"read_iops":  readIOPS,
			"write_iops": writeIOPS,
		},
		"load1":        load1,
		"mem_used_pct": memPercent,
		"top_process":  top,
	}, nil
}

func parseLoad1(text string) any {
	fields := strings.Fields(text)
	if len(fields) == 0 {
		return nil
	}
	value, err := strconv.ParseFloat(fields[0], 64)
	if err != nil {
		return nil
	}
	return value
}

func parseCPUStat(text string) (idle uint64, total uint64, err error) {
	scanner := bufio.NewScanner(strings.NewReader(text))
	for scanner.Scan() {
		line := scanner.Text()
		if !strings.HasPrefix(line, "cpu ") {
			continue
		}
		fields := strings.Fields(line)
		if len(fields) < 5 {
			return 0, 0, errStats("invalid /proc/stat cpu line")
		}
		var values []uint64
		for _, field := range fields[1:] {
			n, convErr := strconv.ParseUint(field, 10, 64)
			if convErr != nil {
				return 0, 0, errStats("invalid /proc/stat counter")
			}
			values = append(values, n)
		}
		// user nice system idle iowait irq softirq steal ...
		idle = values[3]
		if len(values) > 4 {
			idle += values[4]
		}
		for _, n := range values {
			total += n
		}
		if len(values) > 8 {
			// guest and guest_nice are already counted in user/nice
			total -= values[8]
			if len(values) > 9 {
				total -= values[9]
			}
		}
		return idle, total, nil
	}
	return 0, 0, errStats("missing cpu line in /proc/stat")
}

func parseMeminfo(text string) (usedMiB int, totalMiB int, percent float64, err error) {
	var totalKB, availableKB uint64
	var haveTotal, haveAvail bool
	scanner := bufio.NewScanner(strings.NewReader(text))
	for scanner.Scan() {
		line := scanner.Text()
		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}
		switch fields[0] {
		case "MemTotal:":
			totalKB, err = strconv.ParseUint(fields[1], 10, 64)
			if err != nil {
				return 0, 0, 0, errStats("invalid MemTotal")
			}
			haveTotal = true
		case "MemAvailable:":
			availableKB, err = strconv.ParseUint(fields[1], 10, 64)
			if err != nil {
				return 0, 0, 0, errStats("invalid MemAvailable")
			}
			haveAvail = true
		}
	}
	if !haveTotal || !haveAvail || totalKB == 0 {
		return 0, 0, 0, errStats("incomplete /proc/meminfo")
	}
	usedKB := totalKB - availableKB
	totalMiB = int(totalKB / 1024)
	usedMiB = int(usedKB / 1024)
	percent = 100.0 * float64(usedKB) / float64(totalKB)
	return usedMiB, totalMiB, percent, nil
}

func parseDiskstats(text string) (reads uint64, writes uint64) {
	scanner := bufio.NewScanner(strings.NewReader(text))
	for scanner.Scan() {
		fields := strings.Fields(scanner.Text())
		if len(fields) < 8 {
			continue
		}
		name := fields[2]
		if !isWholeDisk(name) {
			continue
		}
		r, errR := strconv.ParseUint(fields[3], 10, 64)
		w, errW := strconv.ParseUint(fields[7], 10, 64)
		if errR != nil || errW != nil {
			continue
		}
		reads += r
		writes += w
	}
	return reads, writes
}

func isWholeDisk(name string) bool {
	for _, prefix := range []string{"loop", "ram", "zram", "fd", "sr", "dm-"} {
		if strings.HasPrefix(name, prefix) {
			return false
		}
	}
	if strings.Contains(name, "nvme") {
		return !strings.Contains(name, "p")
	}
	if len(name) == 0 {
		return false
	}
	last := name[len(name)-1]
	return last < '0' || last > '9'
}

func sampleTopProcess() map[string]any {
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return nil
	}
	self := os.Getpid()
	var bestPID int
	var bestName string
	var bestTicks uint64
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		pid, convErr := strconv.Atoi(entry.Name())
		if convErr != nil || pid <= 0 || pid == self {
			continue
		}
		raw, readErr := os.ReadFile("/proc/" + entry.Name() + "/stat")
		if readErr != nil {
			continue
		}
		name, ticks, ok := parseProcStatCPU(string(raw))
		if !ok || ticks <= bestTicks {
			continue
		}
		bestTicks = ticks
		bestPID = pid
		bestName = name
	}
	if bestPID == 0 {
		return nil
	}
	return map[string]any{"name": bestName, "pid": bestPID, "pcpu": 0.0}
}

func parseProcStatCPU(text string) (name string, ticks uint64, ok bool) {
	start := strings.IndexByte(text, '(')
	end := strings.LastIndexByte(text, ')')
	if start < 0 || end <= start {
		return "", 0, false
	}
	name = text[start+1 : end]
	fields := strings.Fields(text[end+1:])
	// after comm: state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime
	if len(fields) < 13 {
		return "", 0, false
	}
	utime, errU := strconv.ParseUint(fields[11], 10, 64)
	stime, errS := strconv.ParseUint(fields[12], 10, 64)
	if errU != nil || errS != nil {
		return "", 0, false
	}
	return name, utime + stime, true
}

type statsError string

func (e statsError) Error() string { return string(e) }

func errStats(message string) error { return statsError(message) }
