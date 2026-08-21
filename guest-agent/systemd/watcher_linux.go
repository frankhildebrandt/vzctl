//go:build linux

package systemd

import (
	"context"
	"time"

	"github.com/coreos/go-systemd/v22/dbus"
)

func startWatcher(ctx context.Context, manager *Manager) *watcherHandle {
	if !Available() {
		return nil
	}
	watchCtx, cancel := context.WithCancel(ctx)
	go runDBusWatcher(watchCtx, manager)
	return &watcherHandle{cancel: cancel}
}

func runDBusWatcher(ctx context.Context, manager *Manager) {
	backoff := time.Second
	for {
		if ctx.Err() != nil {
			return
		}
		conn, err := dbus.NewSystemConnectionContext(ctx)
		if err != nil {
			sleepOrDone(ctx, backoff)
			if backoff < 30*time.Second {
				backoff *= 2
			}
			continue
		}
		backoff = time.Second
		subCtx, subCancel := context.WithCancel(ctx)
		statusCh, errCh := conn.SubscribeUnitsContext(subCtx, 5*time.Second)
	loop:
		for {
			select {
			case <-ctx.Done():
				subCancel()
				conn.Close()
				return
			case <-errCh:
				break loop
			case changed, ok := <-statusCh:
				if !ok {
					break loop
				}
				appendUnitStatuses(manager, changed)
			}
		}
		subCancel()
		conn.Close()
		sleepOrDone(ctx, backoff)
	}
}

func appendUnitStatuses(manager *Manager, changed map[string]*dbus.UnitStatus) {
	now := time.Now().UTC()
	for name, status := range changed {
		if status == nil {
			continue
		}
		appendWatchedUnitEvent(manager, name, status.LoadState, status.ActiveState, status.SubState, now)
	}
}

func sleepOrDone(ctx context.Context, delay time.Duration) {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
	case <-timer.C:
	}
}
