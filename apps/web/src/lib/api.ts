export class ApiClientError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly details?: unknown
  ) {
    super(message);
  }
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    ...init,
    credentials: "include",
    headers: {
      ...(init.body ? { "content-type": "application/json" } : {}),
      ...init.headers
    }
  });
  if (!response.ok) {
    const payload = await response.json().catch(() => null);
    const message = payload?.error?.message ?? `Request failed (${response.status})`;
    throw new ApiClientError(
      message,
      response.status,
      payload?.error?.code,
      payload?.error?.details
    );
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}
