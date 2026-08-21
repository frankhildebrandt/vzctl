package main

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/frankhildebrandt/vzctl/guest-agent/systemd"
)

func (s *server) systemdManager() *systemd.Manager {
	if s.systemd == nil {
		s.systemd = systemd.NewManager()
		s.systemd.EnsureWatcher(context.Background())
	}
	return s.systemd
}

func handleSystemdStatus(req request, manager *systemd.Manager) response {
	if err := decodeParams(req.Params, &struct{}{}); err != nil {
		return errorResponse(req.ID, "proto", "invalid systemd.status parameters", nil)
	}
	status := manager.Status(context.Background())
	return successResponse(req.ID, map[string]any{
		"available": status.Available,
		"version":   status.Version,
	})
}

func handleSystemdList(ctx context.Context, req request, manager *systemd.Manager) response {
	var params struct {
		Type string `json:"type"`
		All  bool   `json:"all"`
	}
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid systemd.list parameters", nil)
	}
	units, err := manager.List(ctx, params.Type, params.All)
	if errors.Is(err, systemd.ErrUnavailable) {
		return errorResponse(req.ID, "unavailable", "systemd is not available on this guest", nil)
	}
	if err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	encoded, _ := json.Marshal(units)
	var payload []any
	_ = json.Unmarshal(encoded, &payload)
	return successResponse(req.ID, map[string]any{"units": payload})
}

func handleSystemdShow(ctx context.Context, req request, manager *systemd.Manager) response {
	var params struct {
		Unit string `json:"unit"`
	}
	if err := decodeParams(req.Params, &params); err != nil || params.Unit == "" {
		return errorResponse(req.ID, "proto", "invalid systemd.show parameters", nil)
	}
	props, err := manager.Show(ctx, params.Unit)
	if errors.Is(err, systemd.ErrUnavailable) {
		return errorResponse(req.ID, "unavailable", "systemd is not available on this guest", nil)
	}
	if err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	return successResponse(req.ID, map[string]any{"unit": props})
}

func handleSystemdControl(ctx context.Context, req request, manager *systemd.Manager) response {
	var params struct {
		Unit   string `json:"unit"`
		Action string `json:"action"`
	}
	if err := decodeParams(req.Params, &params); err != nil || params.Unit == "" || params.Action == "" {
		return errorResponse(req.ID, "proto", "invalid systemd.control parameters", nil)
	}
	err := manager.Control(ctx, params.Unit, params.Action)
	if errors.Is(err, systemd.ErrUnavailable) {
		return errorResponse(req.ID, "unavailable", "systemd is not available on this guest", nil)
	}
	if err != nil {
		return errorResponse(req.ID, "internal", err.Error(), nil)
	}
	return successResponse(req.ID, map[string]any{
		"unit":   params.Unit,
		"action": params.Action,
		"ok":     true,
	})
}

func handleSystemdEvents(req request, manager *systemd.Manager) response {
	var params struct {
		Since string `json:"since"`
		Limit int    `json:"limit"`
	}
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid systemd.events parameters", nil)
	}
	events, cursor := manager.Events(params.Since, params.Limit)
	encoded, _ := json.Marshal(events)
	var payload []any
	_ = json.Unmarshal(encoded, &payload)
	return successResponse(req.ID, map[string]any{
		"events": payload,
		"cursor": cursor,
	})
}

func systemdCapabilities() []string {
	if systemd.Available() {
		return []string{"systemd"}
	}
	return nil
}
