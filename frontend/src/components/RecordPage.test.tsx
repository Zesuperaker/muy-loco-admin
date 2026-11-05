import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { baseRoutes, currentLocation, mockFetch, postRecord, renderApp } from "../test/utils";

const record = postRecord(7, { meta: { tags: ["a"] }, archived: true });
const recordResponse = { record, labels: { user_id: { "1": "Ann" } } };

describe("RecordPage", () => {
  it("edits a record, sending only changed fields", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { body: recordResponse },
        "PATCH /admin/api/models/posts/records/7": (call) => ({
          body: {
            record: { ...record, values: { ...record.values, ...(call.body as object) } },
            labels: {},
          },
        }),
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");

    expect(await screen.findByRole("heading", { name: "Post 7" })).toBeInTheDocument();
    expect(screen.getByLabelText("Title")).toHaveValue("Post 7");
    expect(screen.getByLabelText("Status")).toHaveValue("draft");
    expect(screen.getByLabelText("Featured")).not.toBeChecked();
    expect(screen.getByLabelText("Archived")).toHaveValue("true");
    expect(screen.getByLabelText("Meta")).toHaveValue('{\n  "tags": [\n    "a"\n  ]\n}');
    expect(screen.getByLabelText("User")).toHaveValue("Ann");
    expect(screen.getByText(new Date("2024-01-02T03:04:05+00:00").toLocaleString())).toBeInTheDocument();

    await user.clear(screen.getByLabelText("Title"));
    await user.type(screen.getByLabelText("Title"), "Renamed");
    await user.click(screen.getByLabelText("Featured"));
    await user.selectOptions(screen.getByLabelText("Status"), "published");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Saved")).toBeInTheDocument();
    const patch = calls.find((c) => c.method === "PATCH");
    expect(patch?.body).toEqual({ title: "Renamed", featured: true, status: "published" });
    expect(await screen.findByRole("heading", { name: "Post 7" })).toBeInTheDocument();
  });

  it("shows client and server validation errors", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { body: recordResponse },
        "PATCH /admin/api/models/posts/records/7": {
          status: 422,
          body: { error: "some fields are invalid", fields: { title: ["title must not be empty"] } },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });

    await user.clear(screen.getByLabelText("Views"));
    await user.type(screen.getByLabelText("Price"), "x");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("must be a decimal number")).toBeInTheDocument();

    await user.clear(screen.getByLabelText("Price"));
    await user.type(screen.getByLabelText("Price"), "1.25");
    await user.clear(screen.getByLabelText("Title"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("title must not be empty")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("some fields are invalid");
  });

  it("creates a record and opens it", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "POST /admin/api/models/posts/records": { status: 201, body: { record: postRecord(9), labels: {} } },
        "GET /admin/api/models/posts/records/9": { body: { record: postRecord(9), labels: {} } },
        "GET /admin/api/models/users/lookup": {
          body: {
            choices: [
              { key: "1", value: 1, label: "Ann" },
              { key: "2", value: 2, label: "Bob" },
            ],
          },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/new");

    expect(await screen.findByRole("heading", { name: "New posts" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Id")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Created at")).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Title"), "Fresh");
    await user.type(screen.getByLabelText("Body"), "Text");
    await user.selectOptions(screen.getByLabelText("Status"), "draft");
    await user.type(screen.getByLabelText("Price"), "3.50");
    await user.type(screen.getByLabelText("Views"), "4");
    await user.type(screen.getByLabelText("Rating"), "2.5");
    fireEvent.change(screen.getByLabelText("Published on"), { target: { value: "2024-02-03" } });
    fireEvent.change(screen.getByLabelText("Remind at"), { target: { value: "10:15:00" } });
    fireEvent.change(screen.getByLabelText("Local at"), { target: { value: "2024-02-03T10:15" } });
    fireEvent.change(screen.getByLabelText("Meta"), { target: { value: '{"a":1}' } });

    const picker = screen.getByLabelText("User");
    await user.click(picker);
    expect(await screen.findByRole("option", { name: /Bob/ })).toBeInTheDocument();
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowUp}{ArrowDown}{Enter}");
    expect(picker).toHaveValue("Bob");

    await user.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(currentLocation()).toBe("/models/posts/records/9"));
    expect(await screen.findByText("Created Post 9")).toBeInTheDocument();
    const post = calls.find((c) => c.method === "POST");
    expect(post?.body).toEqual({
      title: "Fresh",
      body: "Text",
      status: "draft",
      featured: false,
      price: "3.50",
      views: 4,
      rating: 2.5,
      published_on: "2024-02-03",
      remind_at: "10:15:00",
      local_at: "2024-02-03T10:15:00",
      meta: { a: 1 },
      user_id: 2,
    });
  });

  it("deletes after confirmation and links related records", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/models/users/records/1": {
          body: { record: { key: "1", label: "Ann", values: { id: 1, email: "a@x", name: "Ann" } }, labels: {} },
        },
        "GET /admin/api/models/posts/records/7": { body: recordResponse },
        "DELETE /admin/api/models/posts/records/7": { body: { deleted: 1 } },
        "GET /admin/api/models/posts/records": { body: { records: [], total: 0, page: 1, per_page: 25, pages: 0, labels: {} } },
      }),
    );
    const user = userEvent.setup();
    const { unmount } = renderApp("/admin/models/users/records/1");
    expect(await screen.findByRole("heading", { name: "Ann" })).toBeInTheDocument();
    const related = screen.getAllByRole("link", { name: "Posts" }).at(-1);
    expect(related).toHaveAttribute("href", "/admin/models/posts?filter.user_id=1");
    expect(screen.queryByRole("button", { name: "Delete" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("Id")).toHaveTextContent("1");
    unmount();

    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(calls.some((c) => c.method === "DELETE")).toBe(false);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(confirm).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(currentLocation()).toBe("/models/posts"));
    expect(await screen.findByText("Deleted")).toBeInTheDocument();
  });

  it("reports delete failures, missing records and forbidden creation", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/models/posts/records/7": { body: recordResponse },
        "DELETE /admin/api/models/posts/records/7": { status: 422, body: { error: "locked" } },
        "GET /admin/api/models/posts/records/8": { status: 404, body: { error: "not found" } },
      }),
    );
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    const first = renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("locked")).toBeInTheDocument();
    first.unmount();

    const second = renderApp("/admin/models/posts/records/8");
    expect(await screen.findByRole("alert")).toHaveTextContent("not found");
    second.unmount();

    renderApp("/admin/models/users/new");
    expect(await screen.findByRole("alert")).toHaveTextContent("You cannot create users.");
  });

  it("clears nullable foreign keys and closes the picker", async () => {
    const nullableUser = {
      record: postRecord(7),
      labels: { user_id: { "1": "Ann" } },
    };
    mockFetch(
      baseRoutes({
        "GET /admin/api/schema": {
          body: {
            models: [
              (await import("../test/utils")).usersModel,
              {
                ...(await import("../test/utils")).postsModel,
                fields: (await import("../test/utils")).postsModel.fields.map((f) =>
                  f.name === "user_id" ? { ...f, nullable: true } : f,
                ),
              },
            ],
          },
        },
        "GET /admin/api/models/posts/records/7": { body: nullableUser },
        "GET /admin/api/models/users/lookup": { body: { choices: [] } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/posts/records/7");
    await screen.findByRole("heading", { name: "Post 7" });
    await user.click(screen.getByRole("button", { name: "Clear User" }));
    expect(screen.getByLabelText("User")).toHaveValue("");
    await user.click(screen.getByLabelText("User"));
    expect(await screen.findByText("No matches")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByText("No matches")).not.toBeInTheDocument();
  });
});
