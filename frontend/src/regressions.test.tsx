// Regression tests for bugs found in the review before 0.1.0.
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  baseRoutes,
  currentLocation,
  field,
  mockFetch,
  postRecord,
  postsModel,
  renderApp,
  usersModel,
} from "./test/utils";

function bodies(fetchMock: ReturnType<typeof mockFetch>["fetchMock"], method: string) {
  return fetchMock.mock.calls
    .filter(([, init]) => (init?.method ?? "GET").toUpperCase() === method)
    .map(([, init]) => String(init?.body));
}

function pageOf(records: unknown[], extra: Record<string, unknown> = {}) {
  return { records, per_page: 25, next: null, prev: null, labels: {}, ...extra };
}

/** A post as the server writes it, with `user_id` and `meta` as given (raw JSON). */
function postText(userId: string, meta = "null") {
  return (
    '{"record":{"key":"7","label":"Post 7","values":{"id":7,"title":"Post 7","body":"B","status":"draft",' +
    '"featured":false,"archived":null,"rating":null,"price":"1","views":1,"published_on":null,"remind_at":null,' +
    `"meta":${meta},"user_id":${userId},"created_at":"2024-01-02T03:04:05+00:00","local_at":null}},` +
    `"labels":{"user_id":{${JSON.stringify(JSON.parse(userId).toString())}:"Big Ann"}}}`
  );
}

describe("integers beyond 2^53", () => {
  it("are sent exactly as typed", async () => {
    const record = postRecord(7);
    const { fetchMock } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { body: { record, labels: { user_id: { "1": "Ann" } } } },
        "PATCH /admin/api/models/posts/records/7": { body: { record, labels: {} } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    await user.clear(screen.getByLabelText("Views"));
    await user.type(screen.getByLabelText("Views"), "9007199254740993");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(bodies(fetchMock, "PATCH")).toHaveLength(1));
    expect(bodies(fetchMock, "PATCH")[0]).toBe('{"views":"9007199254740993"}');
  });

  it("label and pick foreign keys exactly", async () => {
    const text = postText('"9007199254740993"');
    const { fetchMock } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { text },
        "GET /admin/api/models/users/lookup": {
          text: '{"choices":[{"key":"9007199254740995","value":"9007199254740995","label":"Other Big"}]}',
        },
        "PATCH /admin/api/models/posts/records/7": { text },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    expect(screen.getByLabelText("User")).toHaveValue("Big Ann");
    await user.click(screen.getByLabelText("User"));
    await user.click(await screen.findByRole("option", { name: /Other Big/ }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(bodies(fetchMock, "PATCH")).toHaveLength(1));
    expect(bodies(fetchMock, "PATCH")[0]).toBe('{"user_id":"9007199254740995"}');
  });

  it("inside JSON fields survive an edit", async () => {
    const text = postText("1", '{"n":9007199254740993}');
    const { fetchMock } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { text },
        "PATCH /admin/api/models/posts/records/7": { text },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    const meta = screen.getByLabelText("Meta");
    expect((meta as HTMLTextAreaElement).value).toContain("9007199254740993");
    await user.clear(meta);
    await user.type(meta, '{{"n": 9007199254740993, "m": 1}');
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(bodies(fetchMock, "PATCH")).toHaveLength(1));
    expect(bodies(fetchMock, "PATCH")[0]).toBe('{"meta":{"n":9007199254740993,"m":1}}');
  });
});

describe("list state", () => {
  it("search box follows the URL", async () => {
    mockFetch(baseRoutes({ "GET /admin/api/models/posts/records": { body: pageOf([postRecord(1)]) } }));
    const user = userEvent.setup();
    renderApp("/admin/models/posts?q=rust");
    await screen.findByRole("table");
    expect(screen.getByLabelText("Search")).toHaveValue("rust");
    await user.click(screen.getByRole("link", { name: "Posts" }));
    expect(currentLocation()).toBe("/models/posts");
    expect(screen.getByLabelText("Search")).toHaveValue("");
  });

  it("leaving an unchanged text filter keeps the page", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: pageOf([postRecord(1)], { prev: "1" }) },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts?after=3");
    await screen.findByRole("table");
    await user.click(screen.getByLabelText("Views"));
    await user.click(screen.getByRole("heading", { name: "Posts" }));
    expect(currentLocation()).toBe("/models/posts?after=3");
    await user.type(screen.getByLabelText("Views"), "5");
    await user.click(screen.getByRole("heading", { name: "Posts" }));
    expect(currentLocation()).toBe("/models/posts?filter.views=5");
  });

  it("hides the previous search's count until the new one arrives", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    const { fetchMock } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: pageOf([postRecord(1)]) },
        "GET /admin/api/models/posts/count": (call) => ({
          body: { count: { kind: "exact", value: call.url.includes("q=") ? 1 : 60 } },
        }),
      }),
    );
    const impl = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) => {
      if (String(input).includes("/count?q=")) await gate;
      return impl(input, init);
    });
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    expect(await screen.findByText("60 total")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Search"), "rust{Enter}");
    await waitFor(() => expect(screen.queryByText("60 total")).not.toBeInTheDocument());
    release();
    expect(await screen.findByText("1 total")).toBeInTheDocument();
  });

  it("rows of the previous page cannot be selected while the next one loads", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    const { fetchMock, calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": (call) =>
          call.url.includes("after=2")
            ? { body: pageOf([postRecord(4), postRecord(5)], { prev: "4" }) }
            : { body: pageOf([postRecord(1), postRecord(2)], { next: "2" }) },
        "POST /admin/api/models/posts/bulk-delete": { body: { deleted: 2 } },
      }),
    );
    const impl = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) => {
      if (String(input).includes("after=2")) await gate;
      return impl(input, init);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("link", { name: "Post 1" });
    await user.click(screen.getByLabelText("Select Post 1"));
    await user.click(screen.getByRole("button", { name: "Next" }));
    // Page 1 is still on screen while page 2 loads.
    expect(screen.getByLabelText("Select all")).toBeDisabled();
    expect(screen.getByLabelText("Select Post 1")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Delete selected" })).not.toBeInTheDocument();
    release();
    await screen.findByRole("link", { name: "Post 4" });
    expect(within(screen.getByRole("table")).getByLabelText("Select Post 4")).not.toBeChecked();
    await user.click(screen.getByLabelText("Select all"));
    await user.click(screen.getByRole("button", { name: "Delete selected" }));
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true));
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ keys: ["4", "5"] });
  });

  it("foreign-key links use the server's key encoding", async () => {
    const users = {
      ...usersModel,
      fields: [field("code", { primary_key: true, unique: true }), field("name")],
      primary_key: ["code"],
      list_display: ["code", "name"],
    };
    const posts = {
      ...postsModel,
      fields: postsModel.fields.map((f) =>
        f.name === "user_id"
          ? { ...f, kind: "string" as const, relation: { table: "users", column: "code", model: "users" } }
          : f,
      ),
    };
    mockFetch(
      baseRoutes({
        "GET /admin/api/schema": { body: { models: [users, posts] } },
        "GET /admin/api/models/posts/records": {
          body: pageOf([postRecord(1, { user_id: "a,b%é" })], { labels: { user_id: { "a,b%é": "Ann" } } }),
        },
      }),
    );
    renderApp("/admin/models/posts");
    const link = await screen.findByRole("link", { name: "Ann" });
    expect(link).toHaveAttribute("href", `/admin/models/users/records/${encodeURIComponent("a%2Cb%25%C3%A9")}`);
  });
});

describe("record page", () => {
  it("shows read-only values in full", async () => {
    const long = "x".repeat(200);
    const readOnly = { ...postsModel, capabilities: { ...postsModel.capabilities, update: false } };
    mockFetch(
      baseRoutes({
        "GET /admin/api/schema": { body: { models: [usersModel, readOnly] } },
        "GET /admin/api/models/posts/records/7": {
          body: { record: postRecord(7, { body: long }), labels: { user_id: { "1": "Ann" } } },
        },
      }),
    );
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    expect(document.getElementById("field-body")?.textContent).toBe(long);
  });

  it("shows errors for fields the form does not show", async () => {
    mockFetch(
      baseRoutes({
        "POST /admin/api/models/posts/records": {
          status: 422,
          body: {
            error: "some fields are invalid",
            fields: { created_at: ["is read-only"], __all__: ["dates overlap"] },
          },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/new");
    await screen.findByRole("heading", { name: "New record" });
    await user.type(screen.getByLabelText("Title"), "t");
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(await screen.findByText(/is read-only/)).toBeInTheDocument();
    expect(screen.getByText("dates overlap")).toBeInTheDocument();
  });
});

describe("sign-out", () => {
  it("reports a failure and stays signed in", async () => {
    mockFetch(baseRoutes({ "DELETE /admin/api/session": { status: 500, body: { error: "store down" } } }));
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("heading", { name: "Dashboard" });
    await user.click(screen.getByRole("button", { name: "Sign out" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Sign-out failed: store down");
    expect(screen.getByRole("heading", { name: "Dashboard" })).toBeInTheDocument();
  });
});
