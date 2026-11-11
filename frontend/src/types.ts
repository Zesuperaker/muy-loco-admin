// The API's types are generated from the crate's Rust types into `bindings.ts`.
import type { RecordResponse, RoleWithUsers } from "./bindings";

export type * from "./bindings";

/** Foreign-key field name, then the key value as text, to the target label. */
export type Labels = RecordResponse["labels"];

/** Model name to what the role may do with it; models not listed are hidden. */
export type Permissions = RoleWithUsers["permissions"];
