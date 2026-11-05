import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { baseRoutes, currentLocation, mockFetch, postRecord, renderApp } from "../test/utils";
import { queryFromSearch } from "./ModelList";

function page(records = [postRecord(2), postRecord(1, { featured: true, rating: 4.5 })], extra = {}) {
  return {
    records,
    total: records.length,
    page: 1,
    per_page: 25,
    pages: 1,
    labels: { user_id: { "1": "Ann" } },
    ...extra,
  };
}

describe("queryFromSearch", () => {
  it("parses list state from the URL", () => {
    expect(
      queryFromSearch(new URLSearchParams("page=3&sort=title&order=desc&q=x&filter.status=draft")),
    ).toEqual({ page: 3, sort: "title", order: "desc", q: "x", filters: { status: "draft" } });
    expect(queryFromSearch(new URLSearchParams("page=abc&order=up"))).toEqual({
      page: 1,
      sort: undefined,
      order: undefined,
      q: undefined,
      filters: {},
    });
  });
});

describe("ModelList", () => {
  it("renders records with links, labels and badges", async () => {
    mockFetch(baseRoutes({ "GET /admin/api/models/posts/records": { body: page() } }));
    renderApp("/admin/models/posts");
    const table = await screen.findByRole("table");
    expect(screen.getByRole("heading", { name: "Posts" })).toBeInTheDocument();
    expect(screen.getByText("2 total")).toBeInTheDocument();
    const rows = within(table).getAllByRole("row");
    expect(rows).toHaveLength(3);
    expect(within(rows[1]!).getByRole("link", { name: "2" })).toHaveAttribute(
      "href",
      "/admin/models/posts/records/2",
    );
    expect(within(rows[1]!).getAllByRole("link", { name: "Ann" })[0]).toHaveAttribute(
      "href",
      "/admin/models/users/records/1",
    );
    expect(within(rows[2]!).getByText("Yes")).toBeInTheDocument();
    expect(within(rows[2]!).getByText("4.5")).toBeInTheDocument();
    expect(within(rows[1]!).getAllByText("—").length).toBeGreaterThan(0);
    expect(screen.getByRole("columnheader", { name: /Id/ })).toHaveAttribute("aria-sort", "descending");
    expect(document.title).toBe("Posts · Test Admin");
  });

  it("updates the URL when sorting, searching, filtering and paging", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: page(undefined, { pages: 3, total: 60 }) },
        "GET /admin/api/models/users/lookup": { body: { choices: [{ key: "1", value: 1, label: "Ann" }] } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");

    await user.click(screen.getByRole("button", { name: /Title/ }));
    expect(currentLocation()).toBe("/models/posts?sort=title&order=asc");
    await user.click(screen.getByRole("button", { name: /Title/ }));
    expect(currentLocation()).toBe("/models/posts?sort=title&order=desc");

    await user.type(screen.getByLabelText("Search"), "rust");
    await user.click(screen.getByRole("button", { name: "Search" }));
    expect(currentLocation()).toBe("/models/posts?sort=title&order=desc&q=rust");

    await user.selectOptions(screen.getByLabelText("Status"), "published");
    expect(currentLocation()).toContain("filter.status=published");
    await user.selectOptions(screen.getByLabelText("Featured"), "true");
    expect(currentLocation()).toContain("filter.featured=true");

    const views = screen.getByLabelText("Views");
    await user.type(views, "7{Enter}");
    expect(currentLocation()).toContain("filter.views=7");

    await user.click(screen.getByLabelText("User"));
    await user.click(await screen.findByRole("option", { name: /Ann/ }));
    expect(currentLocation()).toContain("filter.user_id=1");

    await user.click(screen.getByRole("button", { name: "Next" }));
    expect(currentLocation()).toContain("page=2");

    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(currentLocation()).toBe("/models/posts?sort=title&order=desc");
    await waitFor(() =>
      expect(calls.some((c) => c.url.includes("filter.user_id=1"))).toBe(true),
    );
  });

  it("bulk deletes and runs actions on selected rows", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: page() },
        "POST /admin/api/models/posts/bulk-delete": { body: { deleted: 2 } },
        "POST /admin/api/models/posts/actions/publish": (call) =>
          (call.body as { keys: string[] }).keys.length === 1
            ? { body: { message: "published 1 post" } }
            : { status: 422, body: { error: "too many" } },
      }),
    );
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");

    await user.click(screen.getByLabelText("Select Post 2"));
    expect(screen.getByText("1 selected")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Publish" }));
    expect(await screen.findByText("published 1 post")).toBeInTheDocument();
    expect(calls.find((c) => c.url.endsWith("/actions/publish"))?.body).toEqual({ keys: ["2"] });

    await user.click(screen.getByLabelText("Select all"));
    expect(screen.getByText("2 selected")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Publish" }));
    expect(await screen.findByText("too many")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete selected" }));
    expect(confirm).toHaveBeenCalledWith("Delete 2 selected records?");
    expect(await screen.findByText("Deleted 2 records")).toBeInTheDocument();
    expect(calls.find((c) => c.url.endsWith("/bulk-delete"))?.body).toEqual({ keys: ["2", "1"] });

    await user.click(screen.getByLabelText("Select all"));
    await user.click(screen.getByLabelText("Select all"));
    expect(screen.queryByText(/selected$/)).not.toBeInTheDocument();
  });

  it("does nothing when bulk delete is not confirmed", async () => {
    const { calls } = mockFetch(baseRoutes({ "GET /admin/api/models/posts/records": { body: page() } }));
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");
    await user.click(screen.getByLabelText("Select Post 1"));
    await user.click(screen.getByRole("button", { name: "Delete selected" }));
    expect(calls.some((c) => c.url.endsWith("/bulk-delete"))).toBe(false);
  });

  it("exports CSV", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: page() },
        "GET /admin/api/models/posts/export": { text: "id\n1\n" },
      }),
    );
    const createObjectURL = vi.fn(() => "blob:csv");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL, revokeObjectURL }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");
    await user.click(screen.getByRole("button", { name: "Export CSV" }));
    await waitFor(() => expect(click).toHaveBeenCalled());
    expect(createObjectURL).toHaveBeenCalled();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:csv");
  });

  it("reports export failures", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: page() },
        "GET /admin/api/models/posts/export": { status: 403, body: { error: "no export" } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts");
    await screen.findByRole("table");
    await user.click(screen.getByRole("button", { name: "Export CSV" }));
    expect(await screen.findByText("no export")).toBeInTheDocument();
  });

  it("handles empty lists, errors and read-only models", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records": { body: page([]) },
        "GET /admin/api/models/users/records": { status: 500, body: { error: "list broke" } },
      }),
    );
    const { unmount } = renderApp("/admin/models/posts");
    expect(await screen.findByText("No records found.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Add record" })).toHaveAttribute(
      "href",
      "/admin/models/posts/new",
    );
    unmount();

    renderApp("/admin/models/users");
    expect(await screen.findByRole("alert")).toHaveTextContent("list broke");
    expect(screen.queryByRole("button", { name: "Export CSV" })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /Add/ })).not.toBeInTheDocument();
  });
});
