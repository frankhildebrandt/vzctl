package main

import (
	"testing"
	"time"
)

func TestParseCPUStatAndMeminfo(t *testing.T) {
	idle, total, err := parseCPUStat("cpu  10 0 10 70 10 0 0 0 0 0\ncpu0 1 0 1 7 1 0 0 0 0 0\n")
	if err != nil {
		t.Fatal(err)
	}
	if idle != 80 || total != 100 {
		t.Fatalf("idle=%d total=%d", idle, total)
	}

	used, all, percent, err := parseMeminfo("MemTotal: 2048000 kB\nMemAvailable: 1024000 kB\n")
	if err != nil {
		t.Fatal(err)
	}
	if used != 1000 || all != 2000 {
		t.Fatalf("used=%d total=%d", used, all)
	}
	if percent < 49.9 || percent > 50.1 {
		t.Fatalf("percent=%v", percent)
	}
}

func TestParseDiskstatsSkipsPartitionsAndLoop(t *testing.T) {
	reads, writes := parseDiskstats(`
   7       0 loop0 9 0 0 0 0 0 0 0 0 0 0
 253       0 vda 100 0 0 0 40 0 0 0 0 0 0
 253       1 vda1 50 0 0 0 20 0 0 0 0 0 0
 259       0 nvme0n1 10 0 0 0 5 0 0 0 0 0 0
 259       1 nvme0n1p1 3 0 0 0 1 0 0 0 0 0 0
`)
	if reads != 110 || writes != 45 {
		t.Fatalf("reads=%d writes=%d", reads, writes)
	}
}

func TestStatsCollectorDeltaOnSecondSample(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	statCalls := 0
	diskCalls := 0
	c := &statsCollector{
		now: func() time.Time { return now },
		readMeminfo: func() (string, error) {
			return "MemTotal: 1024000 kB\nMemAvailable: 512000 kB\n", nil
		},
		readStat: func() (string, error) {
			statCalls++
			if statCalls == 1 {
				return "cpu  10 0 10 80 0 0 0 0 0 0\n", nil
			}
			return "cpu  35 0 35 130 0 0 0 0 0 0\n", nil
		},
		readDiskstats: func() (string, error) {
			diskCalls++
			if diskCalls == 1 {
				return " 253 0 vda 10 0 0 0 4 0 0 0 0 0 0\n", nil
			}
			return " 253 0 vda 30 0 0 0 14 0 0 0 0 0 0\n", nil
		},
	}

	first, err := c.sample()
	if err != nil {
		t.Fatal(err)
	}
	if first["cpu"].(map[string]any)["percent"] != nil {
		t.Fatalf("first cpu percent = %#v", first["cpu"])
	}
	if first["disk"].(map[string]any)["read_iops"] != nil {
		t.Fatalf("first iops = %#v", first["disk"])
	}

	now = now.Add(2 * time.Second)
	second, err := c.sample()
	if err != nil {
		t.Fatal(err)
	}
	cpu := second["cpu"].(map[string]any)["percent"].(float64)
	if cpu < 49 || cpu > 51 {
		t.Fatalf("cpu percent = %v", cpu)
	}
	read := second["disk"].(map[string]any)["read_iops"].(float64)
	write := second["disk"].(map[string]any)["write_iops"].(float64)
	if read < 9.9 || read > 10.1 || write < 4.9 || write > 5.1 {
		t.Fatalf("iops read=%v write=%v", read, write)
	}
}
