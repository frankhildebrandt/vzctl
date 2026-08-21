package main

import (
	"context"
	"io"
	"net/http"
)

func prepareServiceStream(ctx context.Context, req request, registry *serviceRegistry) (response, *http.Response) {
	params, service, errResp := parseServiceHTTP(req, registry)
	if errResp != nil {
		return *errResp, nil
	}
	httpReq, err := buildServiceRequest(ctx, service.URL, params)
	if err != nil {
		return errorResponse(req.ID, "proto", err.Error(), nil), nil
	}
	httpClient := &http.Client{}
	resp, err := httpClient.Do(httpReq)
	if err != nil {
		return errorResponse(req.ID, "internal", "upstream request failed", nil), nil
	}
	return successResponse(req.ID, map[string]any{
		"upgraded":     true,
		"status":       resp.StatusCode,
		"content_type": resp.Header.Get("Content-Type"),
		"name":         service.Name,
		"kind":         service.Kind,
	}), resp
}

func pipeHTTPStream(writer *responseWriter, resp *http.Response) error {
	defer resp.Body.Close()
	buf := make([]byte, 32*1024)
	for {
		n, err := resp.Body.Read(buf)
		if n > 0 {
			if writeErr := writer.writeMux(muxStdout, buf[:n]); writeErr != nil {
				return writeErr
			}
		}
		if err == io.EOF {
			return writer.writeMux(muxExit, []byte("0"))
		}
		if err != nil {
			_ = writer.writeMux(muxExit, []byte("1"))
			return err
		}
	}
}
