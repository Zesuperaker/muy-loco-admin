import { parseJson } from "./json";
import type {
  AdminConfig,
  AdminRecord,
  AdminUser,
  Choice,
  Count,
  FormLogin,
  JsonValue,
  Labels,
  ModelMeta,
  Page,
  Permissions,
  Role,
} from "./types";

const DEV_BASE = "/admin";

/** The admin mount path, written into index.html by the server. */
export function basePath(doc: Document = document): string {
  const meta = doc.querySelector<HTMLMetaElement>('meta[name="mla-base"]');
  const content = meta?.content ?? "";
  if (!content || content.includes("__MLA_BASE__")) {
    return DEV_BASE;
  }
  return content.replace(/\/+$/, "");
}

export class ApiError extends Error {
  readonly status: number;
  readonly fields: Record<string, string[]>;

  constructor(status: number, message: string, fields: Record<string, string[]> = {}) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.fields = fields;
  }
}

type Listener = () => void;
const unauthorizedListeners = new Set<Listener>();

/** Called whenever the API rejects the session. */
export function onUnauthorized(listener: Listener): () => void {
  unauthorizedListeners.add(listener);
  return () => unauthorizedListeners.delete(listener);
}

async function errorFrom(res: Response): Promise<ApiError> {
  let message = res.statusText || `request failed (${res.status})`;
  let fields: Record<string, string[]> = {};
  try {
    const body = (await res.json()) as { error?: string; fields?: Record<string, string[]> };
    if (body.error) message = body.error;
    if (body.fields) fields = body.fields;
  } catch {
    // Not JSON; keep the status text.
  }
  return new ApiError(res.status, message, fields);
}

/** A request to the admin API. The browser sends the session cookie itself. */
export async function request(path: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("X-Requested-With", "muy-loco-admin");
  if (!headers.has("Accept")) headers.set("Accept", "application/json");
  if (init.body !== undefined && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const res = await fetch(`${basePath()}/api${path}`, { ...init, headers });
  if (!res.ok) {
    if (res.status === 401) {
      unauthorizedListeners.forEach((listener) => listener());
    }
    throw await errorFrom(res);
  }
  return res;
}

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await request(path, init);
  return parseJson(await res.text()) as T;
}

function send<T>(method: string, path: string, body: unknown): Promise<T> {
  return json<T>(path, { method, body: JSON.stringify(body) });
}

const model = (name: string) => `/models/${encodeURIComponent(name)}`;
const record = (name: string, key: string) => `${model(name)}/records/${encodeURIComponent(key)}`;

export interface ListQuery {
  /** The page's number, with numbered pages. */
  page?: number;
  /** Records after the one with this key. */
  after?: string;
  /** Records before the one with this key. */
  before?: string;
  per_page?: number;
  sort?: string;
  order?: "asc" | "desc";
  q?: string;
  filters?: Record<string, string>;
}

export function listSearch(query: ListQuery): string {
  const params = new URLSearchParams();
  if (query.page && query.page > 1) params.set("page", String(query.page));
  if (query.after) params.set("after", query.after);
  else if (query.before) params.set("before", query.before);
  if (query.per_page) params.set("per_page", String(query.per_page));
  if (query.sort) params.set("sort", query.sort);
  if (query.order) params.set("order", query.order);
  if (query.q) params.set("q", query.q);
  for (const [field, value] of Object.entries(query.filters ?? {})) {
    if (value !== "") params.set(`filter.${field}`, value);
  }
  const text = params.toString();
  return text ? `?${text}` : "";
}

export interface RecordResponse {
  record: AdminRecord;
  labels: Labels;
}

export const api = {
  config: () => json<AdminConfig>("/config"),
  me: () => json<AdminUser>("/me"),
  schema: () => json<{ models: ModelMeta[] }>("/schema"),
  stats: () => json<{ counts: Record<string, Count> }>("/stats"),
  list: (name: string, query: ListQuery) => json<Page>(`${model(name)}/records${listSearch(query)}`),
  /** The records matching the list's search and filters. */
  count: (name: string, query: ListQuery) =>
    json<{ count: Count }>(`${model(name)}/count${listSearch({ q: query.q, filters: query.filters })}`),
  get: (name: string, key: string) => json<RecordResponse>(record(name, key)),
  create: (name: string, values: Record<string, JsonValue>) =>
    send<RecordResponse>("POST", `${model(name)}/records`, values),
  update: (name: string, key: string, values: Record<string, JsonValue>) =>
    send<RecordResponse>("PATCH", record(name, key), values),
  remove: (name: string, key: string) =>
    json<{ deleted: number }>(record(name, key), { method: "DELETE" }),
  bulkDelete: (name: string, keys: string[]) =>
    send<{ deleted: number }>("POST", `${model(name)}/bulk-delete`, { keys }),
  action: (name: string, action: string, keys: string[]) =>
    send<{ message: string }>(
      "POST",
      `${model(name)}/actions/${encodeURIComponent(action)}`,
      { keys },
    ),
  lookup: (name: string, q: string, column?: string) => {
    const params = new URLSearchParams({ q });
    if (column) params.set("column", column);
    return json<{ choices: Choice[] }>(`${model(name)}/lookup?${params.toString()}`);
  },
  exportCsv: async (name: string, query: ListQuery): Promise<Blob> => {
    const all = { ...query, page: undefined, after: undefined, before: undefined };
    const res = await request(`${model(name)}/export${listSearch(all)}`, {
      headers: { Accept: "text/csv" },
    });
    return res.blob();
  },
  roles: () => json<{ roles: Role[] }>("/roles"),
  role: (id: number) => json<{ role: Role }>(`/roles/${id}`),
  createRole: (input: { name: string; permissions: Permissions }) =>
    send<{ role: Role }>("POST", "/roles", input),
  updateRole: (id: number, input: { name: string; permissions: Permissions }) =>
    send<{ role: Role }>("PATCH", `/roles/${id}`, input),
  deleteRole: (id: number) => json<{ deleted: number }>(`/roles/${id}`, { method: "DELETE" }),
  /** End the session (the app's too). */
  signOut: async (): Promise<void> => {
    await request("/session", { method: "DELETE" });
  },
};

/**
 * Sign in through the app's login endpoint, then trade the returned token for
 * the admin's session cookie. The token is not kept.
 */
export async function login(
  method: FormLogin,
  identifier: string,
  password: string,
): Promise<void> {
  const res = await fetch(method.endpoint, {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify({
      [method.identifier_field]: identifier,
      [method.password_field]: password,
    }),
  });
  if (!res.ok) {
    throw new ApiError(res.status, res.status === 401 ? "Invalid credentials" : "Sign-in failed");
  }
  const body = (await res.json()) as Record<string, unknown>;
  const token = body[method.token_field];
  if (typeof token !== "string" || !token) {
    throw new ApiError(500, "The login response did not contain a token");
  }
  await request("/session", { method: "POST", headers: { Authorization: `Bearer ${token}` } });
}
