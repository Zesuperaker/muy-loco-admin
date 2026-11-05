import { describe, expect, it, vi } from "vitest";
import { ApiError, api, basePath, getToken, listSearch, login, onUnauthorized, setToken } from "./api";
import { formConfig, mockFetch } from "./test/utils";
import type { FormLogin } from "./types";

const loginMethod = formConfig.login as FormLogin;

function docWithBase(content: string | null): Document {
  const doc = document.implementation.createHTMLDocument("x");
  if (content !== null) {
    const meta = doc.createElement("meta");
    meta.name = "mla-base";
    meta.content = content;
    doc.head.append(meta);
  }
  return doc;
}

describe("basePath", () => {
  it("reads the mount path from the meta tag", () => {
    expect(basePath(docWithBase("/backoffice/"))).toBe("/backoffice");
    expect(basePath(docWithBase("/ops/admin"))).toBe("/ops/admin");
  });

  it("falls back to /admin when unset or unreplaced", () => {
    expect(basePath(docWithBase(null))).toBe("/admin");
    expect(basePath(docWithBase("/__MLA_BASE__"))).toBe("/admin");
    expect(basePath(docWithBase(""))).toBe("/admin");
  });
});

describe("tokens", () => {
  it("stores and clears the token", () => {
    expect(getToken()).toBeNull();
    setToken("abc");
    expect(getToken()).toBe("abc");
    setToken(null);
    expect(getToken()).toBeNull();
  });

  it("survives unavailable storage", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(getToken()).toBeNull();
    expect(() => setToken("x")).not.toThrow();
  });
});

describe("listSearch", () => {
  it("builds query strings", () => {
    expect(listSearch({})).toBe("");
    expect(listSearch({ page: 1 })).toBe("");
    expect(
      listSearch({
        page: 2,
        per_page: 10,
        sort: "title",
        order: "desc",
        q: "a b",
        filters: { status: "draft", empty: "" },
      }),
    ).toBe("?page=2&per_page=10&sort=title&order=desc&q=a+b&filter.status=draft");
  });
});

describe("requests", () => {
  it("sends the CSRF header, token and JSON body", async () => {
    const { calls } = mockFetch({ "POST /admin/api/models/posts/records": { body: { record: {} } } });
    setToken("tok");
    await api.create("posts", { title: "x" });
    const call = calls[0]!;
    expect(call.headers.get("X-Requested-With")).toBe("muy-loco-admin");
    expect(call.headers.get("Authorization")).toBe("Bearer tok");
    expect(call.headers.get("Content-Type")).toBe("application/json");
    expect(call.body).toEqual({ title: "x" });
  });

  it("encodes model names and keys", async () => {
    const { calls } = mockFetch({});
    await api.get("posts", "1,a%2Cb").catch(() => undefined);
    await api.update("post tags", "a/b", {}).catch(() => undefined);
    await api.remove("posts", "7").catch(() => undefined);
    await api.bulkDelete("posts", ["1"]).catch(() => undefined);
    await api.action("posts", "re publish", ["1"]).catch(() => undefined);
    await api.lookup("users", "an", "id").catch(() => undefined);
    await api.lookup("users", "").catch(() => undefined);
    await api.list("posts", { q: "x" }).catch(() => undefined);
    expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      "GET /admin/api/models/posts/records/1%2Ca%252Cb",
      "PATCH /admin/api/models/post%20tags/records/a%2Fb",
      "DELETE /admin/api/models/posts/records/7",
      "POST /admin/api/models/posts/bulk-delete",
      "POST /admin/api/models/posts/actions/re%20publish",
      "GET /admin/api/models/users/lookup?q=an&column=id",
      "GET /admin/api/models/users/lookup?q=",
      "GET /admin/api/models/posts/records?q=x",
    ]);
  });

  it("turns error responses into ApiError", async () => {
    mockFetch({
      "GET /admin/api/me": { status: 422, body: { error: "bad", fields: { a: ["x"] } } },
      "GET /admin/api/schema": { status: 500, text: "oops" },
    });
    const err = await api.me().catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 422, message: "bad", fields: { a: ["x"] } });
    const plain = await api.schema().catch((e: unknown) => e);
    expect(plain).toMatchObject({ status: 500 });
  });

  it("notifies listeners on 401", async () => {
    mockFetch({ "GET /admin/api/me": { status: 401, body: { error: "authentication required" } } });
    const listener = vi.fn();
    const off = onUnauthorized(listener);
    await api.me().catch(() => undefined);
    expect(listener).toHaveBeenCalledTimes(1);
    off();
    await api.me().catch(() => undefined);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("downloads CSV exports", async () => {
    const { calls } = mockFetch({ "GET /admin/api/models/posts/export": { text: "id\n1\n" } });
    const blob = await api.exportCsv("posts", { page: 3, q: "x" });
    expect(await blob.text()).toBe("id\n1\n");
    expect(calls[0]!.url).toBe("/admin/api/models/posts/export?q=x");
    expect(calls[0]!.headers.get("Accept")).toBe("text/csv");
  });
});

describe("login", () => {
  it("posts credentials and stores the token", async () => {
    const { calls } = mockFetch({ "POST /api/auth/login": { body: { token: "jwt" } } });
    await login(loginMethod, "ann@example.com", "pw");
    expect(calls[0]!.body).toEqual({ email: "ann@example.com", password: "pw" });
    expect(getToken()).toBe("jwt");
  });

  it("reports invalid credentials", async () => {
    mockFetch({ "POST /api/auth/login": { status: 401, body: {} } });
    await expect(login(loginMethod, "a", "b")).rejects.toThrow("Invalid credentials");
    mockFetch({ "POST /api/auth/login": { status: 500, body: {} } });
    await expect(login(loginMethod, "a", "b")).rejects.toThrow("Sign-in failed");
  });

  it("requires a token in the response", async () => {
    mockFetch({ "POST /api/auth/login": { body: { pid: "x" } } });
    await expect(login(loginMethod, "a", "b")).rejects.toThrow("did not contain a token");
  });
});
