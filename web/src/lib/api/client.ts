import type { ErrorResponse } from "./schemas";

const API_BASE = import.meta.env.PUBLIC_AXGIT_API_BASE ?? "/api/v1";

export class ApiError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(code: string, message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = status;
  }
}

function isErrorResponse(value: unknown): value is ErrorResponse {
  if (typeof value !== "object" || value === null || !("error" in value)) {
    return false;
  }
  const error = (value as { error: unknown }).error;
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "message" in error &&
    typeof (error as { code: unknown }).code === "string" &&
    typeof (error as { message: unknown }).message === "string"
  );
}

export async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, init);
  } catch {
    throw new ApiError("internal", "network request failed", 0);
  }

  if (response.ok) {
    return (await response.json()) as T;
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    throw new ApiError("internal", response.statusText || "request failed", response.status);
  }

  if (isErrorResponse(body)) {
    throw new ApiError(body.error.code, body.error.message, response.status);
  }

  throw new ApiError("internal", response.statusText || "request failed", response.status);
}
