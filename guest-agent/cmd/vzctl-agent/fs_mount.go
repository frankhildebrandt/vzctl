package main

import (
	"fmt"
	"os"
	"os/exec"
	"path"
	"regexp"
	"strings"
)

const (
	virtiofsDeviceTag = "vzctl"
	virtiofsGuestRoot = "/mnt/vzctl"
	virtiofsBindPath  = "/usr/local/lib/vzctl/virtiofs-bind"
)

var volumeNamePattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]{0,35}$`)

type fsMountParams struct {
	Name     string `json:"name"`
	Target   string `json:"target"`
	ReadOnly bool   `json:"read_only"`
}

type fsUnmountParams struct {
	Name   *string `json:"name"`
	Target *string `json:"target"`
}

func handleFSMount(req request) response {
	var params fsMountParams
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid fs.mount parameters", nil)
	}
	if err := validateFSMountName(params.Name); err != nil {
		return errorResponse(req.ID, "proto", err.Error(), nil)
	}
	if err := validateFSMountTarget(params.Target); err != nil {
		return errorResponse(req.ID, "proto", err.Error(), nil)
	}
	args := []string{virtiofsBindBinary(), "mount", params.Name, params.Target}
	if params.ReadOnly {
		args = append(args, "ro")
	}
	if err := runVirtiofsBind(args); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), map[string]any{
			"name":   params.Name,
			"target": params.Target,
		})
	}
	return successResponse(req.ID, map[string]any{
		"mounted": true,
		"name":    params.Name,
		"target":  params.Target,
	})
}

func handleFSUnmount(req request) response {
	var params fsUnmountParams
	if err := decodeParams(req.Params, &params); err != nil {
		return errorResponse(req.ID, "proto", "invalid fs.unmount parameters", nil)
	}
	name := ""
	if params.Name != nil {
		name = *params.Name
	}
	target := ""
	if params.Target != nil {
		target = *params.Target
	}
	if name == "" && target == "" {
		return errorResponse(req.ID, "proto", "fs.unmount requires name or target", nil)
	}
	if name != "" {
		if err := validateFSMountName(name); err != nil {
			return errorResponse(req.ID, "proto", err.Error(), nil)
		}
	}
	if target != "" {
		if err := validateFSMountTarget(target); err != nil {
			return errorResponse(req.ID, "proto", err.Error(), nil)
		}
	}
	args := []string{virtiofsBindBinary(), "unmount", name, target}
	if err := runVirtiofsBind(args); err != nil {
		return errorResponse(req.ID, "internal", err.Error(), map[string]any{
			"name":   name,
			"target": target,
		})
	}
	return successResponse(req.ID, map[string]any{
		"mounted": false,
		"name":    name,
		"target":  target,
	})
}

func validateFSMountName(name string) error {
	if name == virtiofsDeviceTag {
		return fmt.Errorf("mount name %q is reserved", virtiofsDeviceTag)
	}
	if !volumeNamePattern.MatchString(name) {
		return fmt.Errorf("mount name must match [A-Za-z0-9][A-Za-z0-9_-]* (1-36)")
	}
	return nil
}

func validateFSMountTarget(target string) error {
	if !strings.HasPrefix(target, "/") || target == "/" || strings.Contains(target, "\x00") {
		return fmt.Errorf("mount target must be an absolute path")
	}
	cleaned := path.Clean(target)
	if cleaned != target {
		return fmt.Errorf("mount target must be a cleaned absolute path")
	}
	return nil
}

func virtiofsBindBinary() string {
	if override := strings.TrimSpace(os.Getenv("VZCTL_VIRTIOFS_BIND")); override != "" {
		return override
	}
	return virtiofsBindPath
}

func runVirtiofsBind(argv []string) error {
	cmd := exec.Command("sudo", append([]string{"-n"}, argv...)...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		msg := strings.TrimSpace(string(output))
		if msg == "" {
			msg = err.Error()
		}
		return fmt.Errorf("virtiofs-bind failed: %s", msg)
	}
	return nil
}
