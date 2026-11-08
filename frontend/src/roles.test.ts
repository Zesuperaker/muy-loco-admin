import { describe, expect, it } from "vitest";
import {
  emptyGrant,
  grantsAnything,
  plainIdWarnings,
  summary,
  toggleAction,
  toggleOperation,
} from "./roles";
import { postsModel, usersModel } from "./test/utils";

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
});
