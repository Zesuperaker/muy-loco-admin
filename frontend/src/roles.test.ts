import { describe, expect, it } from "vitest";
import {
  emptyGrant,
  grantsAnything,
  plainIdWarnings,
  summary,
  toggleAction,
  toggleOperation,
  userWarnings,
} from "./roles";
import { field, postsModel, usersModel } from "./test/utils";

/** Permissions granting `grant` on users. */
function users(grant: object) {
  return { users: { ...emptyGrant(), view: true, ...grant } };
}

describe("roles", () => {
  it("grants view with any other permission and removes everything with view", () => {
    let permissions = toggleOperation({}, "posts", "update", true);
    expect(permissions.posts).toEqual({ ...emptyGrant(), view: true, update: true });
    permissions = toggleOperation(permissions, "posts", "update", false);
    expect(permissions.posts).toEqual({ ...emptyGrant(), view: true });
    permissions = toggleAction(permissions, "posts", "publish", true);
    expect(permissions.posts?.actions).toEqual(["publish"]);
    permissions = toggleAction(permissions, "posts", "publish", false);
    expect(permissions.posts?.actions).toEqual([]);
    expect(toggleAction({}, "posts", "publish", true).posts?.view).toBe(true);
    expect(toggleOperation(permissions, "posts", "view", false)).toEqual({});
    expect(toggleOperation({}, "posts", "view", true).posts?.view).toBe(true);
  });

  it("tells what is granted", () => {
    expect(grantsAnything(undefined)).toBe(false);
    expect(grantsAnything(emptyGrant())).toBe(false);
    expect(grantsAnything({ ...emptyGrant(), actions: ["x"] })).toBe(true);
    const models = [usersModel, postsModel];
    expect(summary({}, models)).toBe("Nothing");
    expect(summary({ posts: { ...emptyGrant(), view: true } }, models)).toBe("Posts");
  });

  it("warns about links to models the role cannot view", () => {
    const models = [usersModel, postsModel];
    const edit = { posts: { ...emptyGrant(), view: true, update: true } };
    expect(plainIdWarnings(edit, models)).toEqual([
      "Posts › User will be a plain id field, because this role cannot view Users.",
    ]);
    expect(
      plainIdWarnings({ ...edit, users: { ...emptyGrant(), view: true } }, models),
    ).toEqual([]);
    // Only roles that can write the field are warned.
    expect(plainIdWarnings({ posts: { ...emptyGrant(), view: true } }, models)).toEqual([]);
    // A model that does not offer the write is not warned about.
    const readOnly = { ...postsModel, capabilities: { ...postsModel.capabilities, update: false } };
    expect(plainIdWarnings(edit, [usersModel, readOnly])).toEqual([]);
    // An unregistered target is named by its model name.
    expect(plainIdWarnings(edit, [postsModel])).toEqual([
      "Posts › User will be a plain id field, because this role cannot view users.",
    ]);
  });

  it("warns when the role can edit or delete the admin's users", () => {
    const admins = {
      ...usersModel,
      fields: [...usersModel.fields, field("admin_role_id", { kind: "integer", role: true })],
      capabilities: { ...usersModel.capabilities, delete: true },
    };
    expect(userWarnings(users({ update: true }), [admins, postsModel])).toEqual([
      "This role can edit Users, including admins with full access, for example change their email. Grant it only to people you would trust with full access.",
    ]);
    expect(userWarnings(users({ update: true, delete: true }), [admins])[0]).toContain(
      "can edit and delete Users",
    );
    expect(userWarnings(users({ delete: true }), [admins])[0]).toContain("can delete Users");
    // Viewing is fine, and so are models without the role column.
    expect(userWarnings(users({}), [admins])).toEqual([]);
    expect(userWarnings(users({ update: true }), [usersModel])).toEqual([]);
    // A model that does not offer the write is not warned about.
    expect(userWarnings(users({ delete: true }), [usersModel])).toEqual([]);
  });
});
