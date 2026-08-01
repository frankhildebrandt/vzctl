import { invoke } from "@tauri-apps/api/core";

export type ApiErrorRequest = {
  method: string;
  path: string;
};

export class ApiError extends Error {
  status: number;
  code: string;
  details?: unknown;
  request?: ApiErrorRequest;

  constructor(
    status: number,
    code: string,
    message: string,
    details?: unknown,
    request?: ApiErrorRequest,
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.details = details;
    this.request = request;
  }
}

type InvokeApiResponse = {
  status: number;
  body: string;
  contentType?: string | null;
};

export type ApiRequestOptions = {
  method?: string;
  body?: unknown;
  headers?: Record<string, string>;
  /** Raw body (string) — skips JSON.stringify */
  rawBody?: string;
  contentType?: string;
};

export async function apiRequest<T = unknown>(
  path: string,
  options: ApiRequestOptions = {},
): Promise<T> {
  const { isDemoMode } = await import("@/lib/demo");
  if (isDemoMode()) {
    const { mockApiRequest } = await import("@/lib/demoFixtures");
    return mockApiRequest<T>(path, options);
  }

  const method = (options.method ?? "GET").toUpperCase();
  const headers: Array<[string, string]> = [];
  if (options.headers) {
    for (const [k, v] of Object.entries(options.headers)) {
      headers.push([k, v]);
    }
  }

  let body: string | undefined;
  if (options.rawBody !== undefined) {
    body = options.rawBody;
    headers.push([
      "Content-Type",
      options.contentType ?? "text/plain; charset=utf-8",
    ]);
  } else if (options.body !== undefined) {
    body = JSON.stringify(options.body);
    headers.push(["Content-Type", "application/json"]);
  }

  const requestMeta: ApiErrorRequest = { method, path };

  let response: InvokeApiResponse;
  try {
    response = await invoke<InvokeApiResponse>("api_request", {
      args: {
        method,
        path,
        headers,
        body: body ?? null,
      },
    });
  } catch (err) {
    throw new ApiError(0, "unavailable", String(err), undefined, requestMeta);
  }

  const text = response.body ?? "";
  if (response.status === 204) {
    return undefined as T;
  }

  let parsed: unknown = null;
  if (text) {
    try {
      parsed = JSON.parse(text) as unknown;
    } catch {
      if (response.status < 200 || response.status >= 300) {
        throw new ApiError(
          response.status,
          "internal",
          text || `HTTP ${response.status}`,
          undefined,
          requestMeta,
        );
      }
      return text as T;
    }
  }

  if (response.status < 200 || response.status >= 300) {
    const err = parsed as {
      error?: { code?: string; message?: string; details?: unknown };
    } | null;
    throw new ApiError(
      response.status,
      err?.error?.code ?? "internal",
      err?.error?.message ?? `HTTP ${response.status}`,
      err?.error?.details,
      requestMeta,
    );
  }
  return parsed as T;
}

export function encodeId(id: string): string {
  return encodeURIComponent(id);
}

export const api = {
  get: <T = unknown>(path: string) => apiRequest<T>(path),
  post: <T = unknown>(path: string, body?: unknown) =>
    apiRequest<T>(path, { method: "POST", body }),
  put: <T = unknown>(path: string, body?: unknown) =>
    apiRequest<T>(path, { method: "PUT", body }),
  patch: <T = unknown>(path: string, body?: unknown) =>
    apiRequest<T>(path, { method: "PATCH", body }),
  delete: <T = unknown>(path: string) => apiRequest<T>(path, { method: "DELETE" }),
  putText: <T = unknown>(path: string, text: string, contentType?: string) =>
    apiRequest<T>(path, { method: "PUT", rawBody: text, contentType }),
  getText: async (path: string): Promise<string> => {
    const { isDemoMode } = await import("@/lib/demo");
    if (isDemoMode()) {
      const { mockApiRequest } = await import("@/lib/demoFixtures");
      return mockApiRequest<string>(path, { method: "GET" });
    }
    const response = await invoke<InvokeApiResponse>("api_request", {
      args: { method: "GET", path, headers: [], body: null },
    });
    if (response.status < 200 || response.status >= 300) {
      throw new ApiError(
        response.status,
        "internal",
        response.body || `HTTP ${response.status}`,
        undefined,
        { method: "GET", path },
      );
    }
    return response.body;
  },
};
