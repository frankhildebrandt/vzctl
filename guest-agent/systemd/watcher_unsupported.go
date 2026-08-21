//go:build !linux

package systemd

import "context"

func startWatcher(ctx context.Context, manager *Manager) *watcherHandle {
	return nil
}
