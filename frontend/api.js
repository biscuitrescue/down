// Thin client for the OilsChain backend. Every value shown in the UI comes from these calls.

const TOKEN_KEY = "oilschain.token";

export const session = {
  get token() {
    try { return sessionStorage.getItem(TOKEN_KEY) || ""; } catch { return ""; }
  },
  set token(value) {
    try {
      if (value) sessionStorage.setItem(TOKEN_KEY, value);
      else sessionStorage.removeItem(TOKEN_KEY);
    } catch { /* storage unavailable: token lives only in memory for this page */ }
    memoryToken = value || "";
  },
};
let memoryToken = "";
const currentToken = () => session.token || memoryToken;

export class ApiError extends Error {
  constructor(status, data) {
    super((data && data.error) || `HTTP ${status}`);
    this.status = status;
    this.data = data;
  }
}

/** Call the API. Resolves with { status, data } for 2xx/409 operation responses; throws ApiError otherwise. */
export async function api(method, path, { body, idem, auth = true } = {}) {
  const headers = { Accept: "application/json" };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (auth && currentToken()) headers.Authorization = `Bearer ${currentToken()}`;
  if (idem) headers["Idempotency-Key"] = idem;
  let resp;
  try {
    resp = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  } catch (e) {
    throw new ApiError(0, { error: `Backend unreachable: ${e.message}` });
  }
  let data = null;
  const text = await resp.text();
  try { data = text ? JSON.parse(text) : null; } catch { data = { error: text }; }
  // 409 with an operation body is a recorded rejection (e.g. contract revert), not a transport error.
  if (resp.ok || (resp.status === 409 && data && data.operation_id)) return { status: resp.status, data };
  throw new ApiError(resp.status, data);
}

export function newIdempotencyKey() {
  return (crypto.randomUUID && crypto.randomUUID()) || `k-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

const TERMINAL = new Set(["confirmed", "failed", "reverted"]);

/** Poll an operation until it is confirmed/failed/reverted (or the timeout passes). */
export async function pollOperation(opId, onUpdate, timeoutMs = 180000) {
  const until = Date.now() + timeoutMs;
  let last;
  while (Date.now() < until) {
    const { data } = await api("GET", `/api/operations/${encodeURIComponent(opId)}`);
    last = data;
    onUpdate && onUpdate(data);
    if (TERMINAL.has(data.status)) return data;
    await new Promise((r) => setTimeout(r, 2000));
  }
  return last;
}
