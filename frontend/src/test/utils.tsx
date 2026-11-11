import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { useEffect } from "react";
import { MemoryRouter, useLocation } from "react-router";
import { vi } from "vitest";
import { App } from "../App";
import { ToastProvider } from "../components/ui";
import type { AdminConfig, FieldMeta, ModelMeta } from "../types";

export interface Call {
  method: string;
  url: string;
  body: unknown;
  headers: Headers;
}

type Reply = { status?: number; body?: unknown; text?: string };
type Handler = (call: Call) => Reply | undefined;

/** Install a fetch mock answering `METHOD path` routes; records every call. */
export function mockFetch(routes: Record<string, Reply | Handler>) {
  const calls: Call[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = String(input);
    const method = (init.method ?? "GET").toUpperCase();
    const body = typeof init.body === "string" ? JSON.parse(init.body) : init.body;
    const call = { method, url, body, headers: new Headers(init.headers) };
    calls.push(call);
    const path = url.split("?")[0];
    const route = routes[`${method} ${url}`] ?? routes[`${method} ${path}`];
    const reply = typeof route === "function" ? route(call) : route;
    if (!reply) {
      return new Response(JSON.stringify({ error: "not found" }), { status: 404 });
    }
    const status = reply.status ?? 200;
    if (reply.text !== undefined) {
      return new Response(reply.text, { status });
    }
    return new Response(JSON.stringify(reply.body ?? {}), {
      status,
      headers: { "Content-Type": "application/json" },
    });
  });
  vi.stubGlobal("fetch", fetchMock);
  return { calls, fetchMock };
}

export function field(name: string, overrides: Partial<FieldMeta> = {}): FieldMeta {
  return {
    name,
    label: name.charAt(0).toUpperCase() + name.slice(1).replaceAll("_", " "),
    kind: "string",
    nullable: false,
    primary_key: false,
    auto_increment: false,
    readonly: false,
    unique: false,
    timezone: false,
    ...overrides,
  };
}

export const usersModel: ModelMeta = {
  name: "users",
  label: "Users",
  table: "users",
  primary_key: ["id"],
  fields: [
    field("id", { kind: "integer", primary_key: true, auto_increment: true, readonly: true, unique: true }),
    field("email", { unique: true }),
    field("name"),
  ],
  list_display: ["id", "email", "name"],
  search_fields: ["email", "name"],
  list_filter: [],
  ordering: { field: "id", direction: "desc" },
  per_page: 25,
  pagination: "pages",
  display_field: "name",
  actions: [],
  related: [{ model: "posts", field: "user_id", label: "Posts" }],
  capabilities: { view: true, create: false, update: true, delete: false, export: false },
};

export const postsModel: ModelMeta = {
  name: "posts",
  label: "Posts",
  table: "posts",
  primary_key: ["id"],
  fields: [
    field("id", { kind: "integer", primary_key: true, auto_increment: true, readonly: true, unique: true }),
    field("title", { max_length: 100 }),
    field("body", { kind: "text" }),
    field("status", { kind: "enum", choices: ["draft", "published"] }),
    field("featured", { kind: "boolean" }),
    field("archived", { kind: "boolean", nullable: true }),
    field("rating", { kind: "float", nullable: true }),
    field("price", { kind: "decimal" }),
    field("views", { kind: "integer" }),
    field("published_on", { kind: "date", nullable: true }),
    field("remind_at", { kind: "time", nullable: true }),
    field("meta", { kind: "json", nullable: true }),
    field("user_id", {
      kind: "integer",
      label: "User",
      relation: { table: "users", column: "id", model: "users" },
    }),
    field("created_at", { kind: "date_time", timezone: true, readonly: true }),
    field("local_at", { kind: "date_time", nullable: true }),
  ],
  list_display: ["id", "title", "status", "featured", "rating", "user_id", "created_at"],
  search_fields: ["title", "body"],
  list_filter: ["status", "featured", "user_id", "views"],
  ordering: { field: "id", direction: "desc" },
  per_page: 25,
  pagination: "pages",
  display_field: "title",
  actions: [{ name: "publish", label: "Publish" }],
  related: [],
  capabilities: { view: true, create: true, update: true, delete: true, export: true },
};

export const formConfig: AdminConfig = {
  title: "Test Admin",
  base_path: "/admin",
  site_url: "/",
  login: {
    type: "form",
    endpoint: "/api/auth/login",
    identifier_field: "email",
    identifier_label: "Email",
    password_field: "password",
    token_field: "token",
  },
};

export function postRecord(id: number, overrides: Record<string, unknown> = {}) {
  return {
    key: String(id),
    label: `Post ${id}`,
    values: {
      id,
      title: `Post ${id}`,
      body: "Body",
      status: "draft",
      featured: false,
      archived: null,
      rating: null,
      price: "9.99",
      views: 3,
      published_on: null,
      remind_at: null,
      meta: null,
      user_id: 1,
      created_at: "2024-01-02T03:04:05+00:00",
      local_at: null,
      ...overrides,
    },
  };
}

/** Routes for a signed-in admin with the users and posts models. */
export function baseRoutes(extra: Record<string, Reply | Handler> = {}) {
  return {
    "GET /admin/api/config": { body: formConfig },
    "GET /admin/api/me": { body: { id: "1", name: "Ann" } },
    "GET /admin/api/schema": { body: { models: [usersModel, postsModel] } },
    "GET /admin/api/stats": {
      body: { counts: { users: { kind: "exact", value: 2 }, posts: { kind: "exact", value: 5 } } },
    },
    "GET /admin/api/models/users/count": { body: { count: { kind: "exact", value: 2 } } },
    "GET /admin/api/models/posts/count": { body: { count: { kind: "exact", value: 2 } } },
    ...extra,
  };
}

let lastLocation = "";

function LocationProbe() {
  const location = useLocation();
  useEffect(() => {
    lastLocation = location.pathname + location.search;
  }, [location]);
  return null;
}

export function currentLocation(): string {
  return lastLocation;
}

export function renderApp(path = "/admin") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter basename="/admin" initialEntries={[path]} useTransitions={false}>
        <ToastProvider>
          <App />
          <LocationProbe />
        </ToastProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}
