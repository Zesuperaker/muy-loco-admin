// Regression tests for bugs found in reviews.
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { hasRawJson, parseJson } from "./json";
import { shareData } from "./queryClient";
import { emptyGrant } from "./roles";
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
  return { records, per_page: 25, page: 1, next: null, prev: null, labels: {}, ...extra };
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
        "GET /admin/api/models/posts/records": {
          body: pageOf([postRecord(1)], { page: 3, prev: "2" }),
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts?page=3");
    await screen.findByRole("table");
    await user.click(screen.getByLabelText("Views"));
    await user.click(screen.getByRole("heading", { name: "Posts" }));
    expect(currentLocation()).toBe("/models/posts?page=3");
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
          call.url.includes("page=2")
            ? { body: pageOf([postRecord(4), postRecord(5)], { page: 2, prev: "1" }) }
            : { body: pageOf([postRecord(1), postRecord(2)], { next: "2" }) },
        "POST /admin/api/models/posts/bulk-delete": { body: { deleted: 2 } },
      }),
    );
    const impl = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) => {
      if (String(input).includes("page=2")) await gate;
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

describe("forms opened from the cache", () => {
  it("a record's fields follow the fresh copy when it arrives", async () => {
    let title = "Post 7";
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: pageOf([postRecord(7)]) },
        "GET /admin/api/models/posts/records/7": () => ({
          body: { record: postRecord(7, { title, status: title === "Post 7" ? "draft" : "published" }), labels: {} },
        }),
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    expect(screen.getByLabelText("Title")).toHaveValue("Post 7");

    // The record changes while the list is open; reopening it shows the
    // cached copy first, then the fresh one.
    await user.click(screen.getByRole("link", { name: "← Posts" }));
    await screen.findByRole("table");
    title = "Changed elsewhere";
    await user.click(screen.getByRole("link", { name: "Post 7" }));
    await waitFor(() => expect(screen.getByLabelText("Title")).toHaveValue("Changed elsewhere"));
    expect(screen.getByLabelText("Status")).toHaveValue("published");
  });

  it("an edited field keeps its text when the record is fetched again", async () => {
    let views = 3;
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": () => ({
          body: { record: postRecord(7, { views }), labels: {} },
        }),
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "Mine");

    // Coming back online fetches the record again under the open form.
    views = 40;
    window.dispatchEvent(new Event("offline"));
    window.dispatchEvent(new Event("online"));
    await waitFor(() => expect(screen.getByLabelText("Views")).toHaveValue("40"));
    expect(screen.getByLabelText("Title")).toHaveValue("Mine");
  });

  it("a role's permissions follow the fresh copy, and a save sends only what was edited", async () => {
    const manager = { id: "1", name: "Ann", role: null, can_manage_roles: true };
    let grant = { ...emptyGrant(), view: true };
    const role = () => ({ id: 3, name: "Editor", permissions: { posts: grant }, users: 0 });
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles": () => ({ body: { roles: [role()] } }),
        "GET /admin/api/roles/3": () => ({ body: { role: role() } }),
        "PATCH /admin/api/roles/3": (call) => ({ body: { role: { ...role(), ...(call.body as object) } } }),
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/roles/3");
    await screen.findByRole("heading", { name: "Editor" });
    expect(screen.getByLabelText("Posts: Delete")).not.toBeChecked();

    // Another admin lets the role delete posts while the list is open.
    await user.click(screen.getByRole("link", { name: "← Roles" }));
    await screen.findByRole("heading", { name: "Roles" });
    grant = { ...grant, delete: true };
    await user.click(await screen.findByRole("link", { name: "Editor" }));
    await waitFor(() => expect(screen.getByLabelText("Posts: Delete")).toBeChecked());

    // Renaming the role leaves its permissions to the server.
    await user.type(screen.getByLabelText("Name"), "s");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({ name: "Editors" });
  });
});

describe("input the browser cannot read", () => {
  it("is refused instead of saved as no value", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": {
          body: { record: postRecord(7, { rating: 2.5, published_on: "2024-05-06" }), labels: {} },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });

    await user.clear(screen.getByLabelText("Rating"));
    await user.type(screen.getByLabelText("Rating"), "1e");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("must be a number")).toBeInTheDocument();

    // Half a date: the browser reports the input as empty and its text as bad.
    await user.clear(screen.getByLabelText("Rating"));
    await user.type(screen.getByLabelText("Rating"), "3.5");
    const date = screen.getByLabelText("Published on");
    fireEvent.change(date, { target: { value: "" } });
    Object.defineProperty(date, "validity", { value: { badInput: true } });
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("is incomplete or not valid")).toBeInTheDocument();
    expect(calls.filter((c) => c.method === "PATCH")).toHaveLength(0);
  });
});

const meta = () => screen.getByLabelText<HTMLTextAreaElement>("Meta");

describe("integers beyond 2^53 in JSON fields", () => {
  it("are kept by the query cache when the field changes", async () => {
    let n = "9007199254740993";
    const { fetchMock } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: pageOf([postRecord(7)]) },
        "GET /admin/api/models/posts/records/7": () => ({ text: postText("1", `{"n":${n}}`) }),
        "PATCH /admin/api/models/posts/records/7": () => {
          n = "9007199254740995";
          return { text: postText("1", `{"n":${n}}`) };
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    await user.clear(meta());
    await user.type(meta(), '{{"n": 9007199254740995}');
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(bodies(fetchMock, "PATCH")[0]).toBe('{"meta":{"n":9007199254740995}}');
    expect(meta().value).toBe('{\n  "n": 9007199254740995\n}');

    await user.click(screen.getByRole("link", { name: "← Posts" }));
    await screen.findByRole("table");
    await user.click(screen.getByRole("link", { name: "Post 7" }));
    await screen.findByRole("heading", { name: "Post 7" });
    expect(meta().value).toBe('{\n  "n": 9007199254740995\n}');
  });

  it("stop structural sharing, which would copy them into plain objects", () => {
    const previous = parseJson('{"a":{"n":9007199254740993},"b":[1]}');
    const next = parseJson('{"a":{"n":9007199254740995},"b":[1]}');
    expect(hasRawJson(next)).toBe(true);
    expect(JSON.stringify(shareData(previous, next))).toBe('{"a":{"n":9007199254740995},"b":[1]}');
    // Without them, unchanged parts are still shared.
    const plain = { a: { n: 1 }, b: [1] };
    const shared = shareData(plain, { a: { n: 2 }, b: [1] }) as typeof plain;
    expect(hasRawJson(plain)).toBe(false);
    expect(shared.a).toEqual({ n: 2 });
    expect(shared.b).toBe(plain.b);
  });
});

describe("list filters", () => {
  it("keep the keyboard focus when they change", async () => {
    mockFetch(baseRoutes({ "GET /admin/api/models/posts/records": { body: pageOf([postRecord(1)]) } }));
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");

    await user.selectOptions(screen.getByLabelText("Status"), "published");
    expect(currentLocation()).toBe("/models/posts?filter.status=published");
    expect(screen.getByLabelText("Status")).toHaveFocus();

    await user.type(screen.getByLabelText("Views"), "5{Enter}");
    expect(currentLocation()).toBe("/models/posts?filter.status=published&filter.views=5");
    expect(screen.getByLabelText("Views")).toHaveFocus();

    // A filter cleared another way empties its field.
    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(currentLocation()).toBe("/models/posts");
    expect(screen.getByLabelText("Views")).toHaveValue("");
  });

  it("show a filter the model does not list, such as one from a Related link", async () => {
    const tags = { ...usersModel, name: "tags", label: "Tags", search_fields: [], list_filter: [] };
    mockFetch(
      baseRoutes({
        "GET /admin/api/schema": { body: { models: [tags] } },
        "GET /admin/api/models/tags/records": { body: pageOf([]) },
        "GET /admin/api/models/tags/count": { body: { count: { kind: "exact", value: 0 } } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/tags");
    await screen.findByRole("table");
    expect(screen.queryByLabelText("Name")).not.toBeInTheDocument();

    renderApp("/admin/models/tags?filter.name=rust");
    const name = await screen.findByLabelText("Name");
    expect(name).toHaveValue("rust");
    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(currentLocation()).toBe("/models/tags");
  });
});

describe("the page title", () => {
  it("is reset on pages without one of their own", async () => {
    mockFetch(baseRoutes({ "GET /admin/api/models/posts/records": { body: pageOf([postRecord(1)]) } }));
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");
    expect(document.title).toBe("Posts · Test Admin");
    await user.click(screen.getByRole("link", { name: "Test Admin" }));
    await screen.findByRole("heading", { name: "Dashboard" });
    expect(document.title).toBe("Test Admin");
  });
});

describe("a rejected form", () => {
  it("tells assistive technology which field to fix, and drops the last attempt's error", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { body: { record: postRecord(7), labels: {} } },
        "PATCH /admin/api/models/posts/records/7": {
          status: 422,
          body: { error: "some fields are invalid", fields: { title: ["title must not be empty"] } },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    const title = screen.getByLabelText("Title");
    expect(title).toHaveAttribute("aria-required", "true");
    expect(screen.getByLabelText("Rating")).not.toHaveAttribute("aria-required");
    expect(title).not.toHaveAttribute("aria-invalid");

    await user.clear(title);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("some fields are invalid");
    await waitFor(() => expect(title).toHaveFocus());
    expect(title).toHaveAttribute("aria-invalid", "true");
    expect(title).toHaveAccessibleDescription("title must not be empty");

    // The next attempt fails in the form itself: the server's error is gone.
    await user.type(screen.getByLabelText("Price"), "x");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("must be a decimal number")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Price")).toHaveFocus();
    expect(title).not.toHaveAttribute("aria-invalid");
  });
});

describe("a save that fails without naming a field", () => {
  it("leaves no field marked from the attempt before", async () => {
    let online = true;
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { body: { record: postRecord(7), labels: {} } },
        "PATCH /admin/api/models/posts/records/7": () => {
          if (!online) throw new TypeError("Failed to fetch");
          return {
            status: 422,
            body: { error: "some fields are invalid", fields: { title: ["title is taken"] } },
          };
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    const title = screen.getByLabelText("Title");
    await user.type(title, "!");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(title).toHaveAttribute("aria-invalid", "true"));

    online = false;
    await user.type(title, "?");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Failed to fetch");
    expect(title).not.toHaveAttribute("aria-invalid");
    expect(screen.queryByText("title is taken")).not.toBeInTheDocument();
  });
});
