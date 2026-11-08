import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { emptyGrant } from "../roles";
import { baseRoutes, currentLocation, field, mockFetch, renderApp, usersModel } from "../test/utils";
import type { ModelMeta, Role } from "../types";

const manager = { id: "1", name: "Ann", role: null, can_manage_roles: true };

const editor: Role = {
  id: 3,
  name: "Editor",
  permissions: { posts: { ...emptyGrant(), view: true, update: true } },
  users: 2,
};
const support: Role = { id: 4, name: "Support", permissions: {}, users: 0 };

const usersWithRoles: ModelMeta = {
  ...usersModel,
  fields: [
    ...usersModel.fields,
    field("admin_role_id", { kind: "integer", nullable: true, role: true, label: "Admin role" }),
  ],
  list_display: [...usersModel.list_display, "admin_role_id"],
  list_filter: ["admin_role_id"],
};

const userRecord = (role: number | null) => ({
  key: "1",
  label: "Ann",
  values: { id: 1, email: "ann@example.com", name: "Ann", admin_role_id: role },
});

describe("Roles", () => {
  it("lists roles for users who manage them", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles": { body: { roles: [editor, support] } },
      }),
    );
    const user = userEvent.setup();
    renderApp();
    await user.click(await screen.findByRole("link", { name: "Roles" }));
    expect(currentLocation()).toBe("/roles");
    expect(await screen.findByRole("heading", { name: "Roles" })).toBeInTheDocument();
    const rows = screen.getAllByRole("row");
    expect(rows[1]).toHaveTextContent("EditorPosts2");
    expect(rows[2]).toHaveTextContent("SupportNothing0");
    expect(document.title).toBe("Roles · Test Admin");
    await user.click(screen.getByRole("link", { name: "Add role" }));
    expect(currentLocation()).toBe("/roles/new");
  });

  it("says when there are no roles or they cannot load", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles": { body: { roles: [] } },
      }),
    );
    renderApp("/admin/roles");
    expect(await screen.findByText("No roles yet.")).toBeInTheDocument();
  });

  it("shows why roles cannot load", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles": { status: 500, body: { error: "internal server error" } },
      }),
    );
    renderApp("/admin/roles");
    expect(await screen.findByRole("alert")).toHaveTextContent("internal server error");
  });

  it("shows a role that does not exist as an error", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles/9": { status: 404, body: { error: "not found" } },
      }),
    );
    renderApp("/admin/roles/9");
    expect(await screen.findByRole("alert")).toHaveTextContent("not found");
  });

  it("creates a role, warning about links it cannot follow", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "POST /admin/api/roles": (call) => ({
          status: 201,
          body: { role: { id: 5, users: 0, ...(call.body as object) } },
        }),
        "GET /admin/api/roles/5": { body: { role: { ...editor, id: 5, users: 0 } } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/roles/new");
    expect(await screen.findByRole("heading", { name: "New role" })).toBeInTheDocument();
    await user.type(screen.getByLabelText("Name"), "Editor");

    // Users cannot be created in this admin at all.
    expect(screen.getByLabelText("Users: Create")).toBeDisabled();
    await user.click(screen.getByLabelText("Posts: Edit"));
    expect(screen.getByLabelText("Posts: View")).toBeChecked();
    expect(screen.getByRole("list", { name: "Warnings" })).toHaveTextContent(
      "Posts › User will be a plain id field, because this role cannot view Users.",
    );
    await user.click(screen.getByLabelText("Users: View"));
    expect(screen.queryByRole("list", { name: "Warnings" })).not.toBeInTheDocument();
    await user.click(screen.getByLabelText("Users: View"));
    await user.click(screen.getByLabelText("Posts: Publish"));
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(currentLocation()).toBe("/roles/5"));
    expect(await screen.findByText("Created Editor")).toBeInTheDocument();
    const post = calls.find((c) => c.method === "POST");
    expect(post?.body).toEqual({
      name: "Editor",
      permissions: {
        posts: { ...emptyGrant(), view: true, update: true, actions: ["publish"] },
      },
    });
  });

  it("shows the server's validation errors", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "POST /admin/api/roles": {
          status: 422,
          body: {
            error: "some fields are invalid",
            fields: { name: ["is already taken"], permissions: ["unknown model `x`"] },
          },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/roles/new");
    await user.type(await screen.findByLabelText("Name"), "Editor");
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(await screen.findByText("is already taken")).toBeInTheDocument();
    expect(screen.getByText("unknown model `x`")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("some fields are invalid");
  });

  it("edits a role and cannot delete it while users have it", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles/3": { body: { role: editor } },
        "PATCH /admin/api/roles/3": (call) => ({
          body: { role: { ...editor, ...(call.body as object) } },
        }),
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/roles/3");
    expect(await screen.findByRole("heading", { name: "Editor" })).toBeInTheDocument();
    expect(screen.getByText("2 users have this role")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
    expect(screen.getByLabelText("Posts: Edit")).toBeChecked();

    await user.clear(screen.getByLabelText("Name"));
    await user.type(screen.getByLabelText("Name"), "Writer");
    await user.click(screen.getByLabelText("Posts: View"));
    expect(screen.getByLabelText("Posts: Edit")).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({
      name: "Writer",
      permissions: {},
    });
  });

  it("says how many users have a role", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles/4": { body: { role: { ...support, users: 1 } } },
      }),
    );
    renderApp("/admin/roles/4");
    expect(await screen.findByText("1 user has this role")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
  });

  it("deletes a role nobody has", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles/4": { body: { role: support } },
        "DELETE /admin/api/roles/4": { body: { deleted: 1 } },
        "GET /admin/api/roles": { body: { roles: [] } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/roles/4");
    expect(await screen.findByText("0 users have this role")).toBeInTheDocument();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(confirm).toHaveBeenCalledWith("Delete the role Support?");
    expect(calls.some((c) => c.method === "DELETE")).toBe(false);
    confirm.mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(currentLocation()).toBe("/roles"));
    expect(await screen.findByText("Deleted")).toBeInTheDocument();
  });

  it("reports a role the server refuses to delete", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/roles/4": { body: { role: support } },
        "DELETE /admin/api/roles/4": {
          status: 409,
          body: { error: "this role is assigned to 1 user; give them another role first" },
        },
      }),
    );
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const user = userEvent.setup();
    renderApp("/admin/roles/4");
    await user.click(await screen.findByRole("button", { name: "Delete" }));
    expect(await screen.findByText(/assigned to 1 user/)).toBeInTheDocument();
  });

  it("hides roles from users who cannot manage them", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": {
          body: { id: "2", name: "Ed", role: { id: 3, name: "Editor" }, can_manage_roles: false },
        },
      }),
    );
    renderApp("/admin/roles");
    expect(await screen.findByRole("heading", { name: "Not found" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Roles" })).not.toBeInTheDocument();
    expect(screen.getByTitle("Your role")).toHaveTextContent("Editor");
  });

  it("assigns roles on a user's record", async () => {
    const { calls } = mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/schema": { body: { models: [usersWithRoles] } },
        "GET /admin/api/roles": { body: { roles: [editor, support] } },
        "GET /admin/api/models/users/records/1": {
          body: { record: userRecord(3), labels: { admin_role_id: { "3": "Editor" } } },
        },
        "PATCH /admin/api/models/users/records/1": (call) => ({
          body: {
            record: userRecord((call.body as { admin_role_id: number | null }).admin_role_id),
            labels: {},
          },
        }),
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/users/records/1");
    const select = await screen.findByLabelText("Admin role");
    await waitFor(() => expect(select).toHaveValue("3"));
    expect(screen.getByText("Admins without a role have full access.")).toBeInTheDocument();

    await user.selectOptions(select, "4");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Admin role"), "No role (full access)");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls.filter((c) => c.method === "PATCH")).toHaveLength(2));
    expect(calls.filter((c) => c.method === "PATCH").map((c) => c.body)).toEqual([
      { admin_role_id: 4 },
      { admin_role_id: null },
    ]);
  });

  it("warns when a role can edit the admin's users", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/schema": { body: { models: [usersWithRoles] } },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/roles/new");
    await user.click(await screen.findByLabelText("Users: Edit"));
    expect(screen.getByRole("list", { name: "Warnings" })).toHaveTextContent(
      "This role can edit Users, including admins with full access",
    );
  });

  it("keeps showing a user's role when the roles cannot load", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/schema": { body: { models: [usersWithRoles] } },
        "GET /admin/api/roles": { status: 500, body: { error: "boom" } },
        "GET /admin/api/models/users/records/1": {
          body: { record: userRecord(3), labels: { admin_role_id: { "3": "Editor" } } },
        },
      }),
    );
    renderApp("/admin/models/users/records/1");
    expect(await screen.findByText("The roles could not be loaded.")).toBeInTheDocument();
    const select = screen.getByLabelText("Admin role");
    expect(select).toHaveValue("3");
    expect(within(select).getByRole("option", { selected: true })).toHaveTextContent("Editor");
  });

  it("shows and filters users by role", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": { body: manager },
        "GET /admin/api/schema": { body: { models: [usersWithRoles] } },
        "GET /admin/api/roles": { body: { roles: [editor, support] } },
        "GET /admin/api/models/users/records": {
          body: {
            records: [userRecord(3), { ...userRecord(null), key: "2" }],
            total: 2,
            page: 1,
            per_page: 25,
            pages: 1,
            labels: { admin_role_id: { "3": "Editor" } },
          },
        },
      }),
    );
    const user = userEvent.setup();
    renderApp("/admin/models/users");
    const table = await screen.findByRole("table");
    expect(await within(table).findByText("Editor")).toBeInTheDocument();

    const filter = screen.getByLabelText("Admin role");
    await waitFor(() => expect(filter).toBeEnabled());
    await user.selectOptions(filter, "Editor");
    await waitFor(() => expect(currentLocation()).toBe("/models/users?filter.admin_role_id=3"));
    await user.selectOptions(screen.getByLabelText("Admin role"), "No role");
    await waitFor(() =>
      expect(currentLocation()).toBe("/models/users?filter.admin_role_id=__null__"),
    );
  });

  it("shows the role field as text to users who cannot assign roles", async () => {
    mockFetch(
      baseRoutes({
        "GET /admin/api/me": {
          body: { id: "2", name: "Sam", role: { id: 4, name: "Support" }, can_manage_roles: false },
        },
        "GET /admin/api/schema": {
          body: {
            models: [
              {
                ...usersWithRoles,
                fields: usersWithRoles.fields.map((f) =>
                  f.role ? { ...f, readonly: true } : f,
                ),
              },
            ],
          },
        },
        "GET /admin/api/models/users/records/1": {
          body: { record: userRecord(3), labels: { admin_role_id: { "3": "Editor" } } },
        },
        "GET /admin/api/models/users/records": {
          body: { records: [], total: 0, page: 1, per_page: 25, pages: 0, labels: {} },
        },
      }),
    );
    renderApp("/admin/models/users/records/1");
    expect(await screen.findByText("Editor")).toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Admin role" })).not.toBeInTheDocument();
  });
});
