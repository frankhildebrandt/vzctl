package main

import (
	"context"
	"encoding/base64"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const maxServiceHTTPBytes = 256 * 1024

type servicesHTTPParams struct {
	Name    string            `json:"name"`
	Method  string            `json:"method"`
	Path    string            `json:"path"`
	Headers map[string]string `json:"headers"`
	BodyB64 string            `json:"body_b64"`
}

func handleServicesList(req request, registry *serviceRegistry) response {
	if err := decodeParams(req.Params, &struct{}{}); err != nil {
		return errorResponse(req.ID, "proto", "invalid services.list parameters", nil)
	}
	if registry == nil {
		registry = newServiceRegistry()
	}
	return successResponse(req.ID, map[string]any{"services": registry.list()})
}

func handleServicesHTTP(ctx context.Context, req request, registry *serviceRegistry) response {
	params, service, errResp := parseServiceHTTP(req, registry)
	if errResp != nil {
		return *errResp
	}
	httpReq, err := buildServiceRequest(ctx, service.URL, params)
	if err != nil {
		return errorResponse(req.ID, "proto", err.Error(), nil)
	}
	resp, err := httpClientWithTimeout().Do(httpReq)
	if err != nil {
		return errorResponse(req.ID, "internal", "upstream request failed", nil)
	}
	defer resp.Body.Close()
	body, truncated, err := readCapped(resp.Body, maxServiceHTTPBytes)
	if err != nil {
		return errorResponse(req.ID, "internal", "upstream read failed", nil)
	}
	return successResponse(req.ID, map[string]any{
		"status":    resp.StatusCode,
		"headers":   flattenHeaders(resp.Header),
		"body_b64":  base64.StdEncoding.EncodeToString(body),
		"truncated": truncated,
		"name":      service.Name,
		"kind":      service.Kind,
	})
}

func parseServiceHTTP(req request, registry *serviceRegistry) (servicesHTTPParams, publishedService, *response) {
	var params servicesHTTPParams
	if err := decodeParams(req.Params, &params); err != nil {
		resp := errorResponse(req.ID, "proto", "invalid services.http parameters", nil)
		return params, publishedService{}, &resp
	}
	if registry == nil {
		resp := errorResponse(req.ID, "not_found", "service not found", map[string]any{"name": params.Name})
		return params, publishedService{}, &resp
	}
	service, ok := registry.get(params.Name)
	if !ok {
		resp := errorResponse(req.ID, "not_found", "service not found", map[string]any{"name": params.Name})
		return params, publishedService{}, &resp
	}
	if err := validateServicePath(params.Path); err != nil {
		resp := errorResponse(req.ID, "proto", err.Error(), nil)
		return params, publishedService{}, &resp
	}
	if params.Method == "" {
		params.Method = http.MethodGet
	}
	return params, service, nil
}

func validateServicePath(raw string) error {
	if raw == "" || !strings.HasPrefix(raw, "/") || strings.HasPrefix(raw, "//") {
		return fmt.Errorf("path must be a root-relative URL path")
	}
	parsed, err := url.Parse(raw)
	if err != nil || parsed.Scheme != "" || parsed.Host != "" {
		return fmt.Errorf("path must not include a host")
	}
	return nil
}

func buildServiceRequest(ctx context.Context, origin string, params servicesHTTPParams) (*http.Request, error) {
	target, err := url.Parse(origin)
	if err != nil {
		return nil, fmt.Errorf("invalid origin")
	}
	rel, err := url.Parse(params.Path)
	if err != nil {
		return nil, fmt.Errorf("invalid path")
	}
	resolved := target.ResolveReference(rel)
	if err := validateLoopbackURL(resolved.String()); err != nil {
		return nil, err
	}
	var body io.Reader
	if params.BodyB64 != "" {
		decoded, err := base64.StdEncoding.DecodeString(params.BodyB64)
		if err != nil {
			return nil, fmt.Errorf("invalid body_b64")
		}
		body = strings.NewReader(string(decoded))
	}
	req, err := http.NewRequestWithContext(ctx, params.Method, resolved.String(), body)
	if err != nil {
		return nil, err
	}
	for key, value := range params.Headers {
		req.Header.Set(key, value)
	}
	return req, nil
}

func flattenHeaders(header http.Header) map[string]string {
	out := map[string]string{}
	for key, values := range header {
		if len(values) > 0 {
			out[key] = values[0]
		}
	}
	return out
}

func readCapped(r io.Reader, limit int) ([]byte, bool, error) {
	data, err := io.ReadAll(io.LimitReader(r, int64(limit)+1))
	if err != nil {
		return nil, false, err
	}
	if len(data) > limit {
		return data[:limit], true, nil
	}
	return data, false, nil
}

func httpClientWithTimeout() *http.Client {
	return &http.Client{Timeout: 15 * time.Second}
}
