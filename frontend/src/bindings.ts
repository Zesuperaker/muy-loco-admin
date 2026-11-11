// Generated from the crate's Rust types by `src/bindings.rs`; do not edit.
// `cargo test` regenerates it.

/**
 * A custom bulk action.
 */
export type ActionMeta = { name: string, label: string, };

/**
 * What a custom action reported.
 */
export type ActionResponse = { message: string, };

/**
 * What the UI needs before anyone signs in.
 */
export type AdminConfig = { title: string,
/**
 * The admin's mount path.
 */
base_path: string,
/**
 * Target of the "Visit site" link; `null` hides it.
 */
site_url: string | null, login: LoginMethod, };

/**
 * One record as sent to the UI.
 */
export type AdminRecord = {
/**
 * Encoded primary key, used in URLs.
 */
key: string,
/**
 * Human-readable label.
 */
label: string,
/**
 * Column name to value.
 */
values: { [key in string]: JsonValue }, };

/**
 * Operations a model supports. A user with a role may do only part of them.
 */
export type Capabilities = { view: boolean, create: boolean, update: boolean, delete: boolean, export: boolean, };

/**
 * An option offered by a foreign-key picker.
 */
export type Choice = { key: string, value: JsonValue, label: string, };

/**
 * The signed-in user as the UI shows them.
 */
export type CurrentUser = { id: string, name: string,
/**
 * The user's role; `null` for full access.
 */
role: RoleRef | null,
/**
 * Roles are on and the user has full access.
 */
can_manage_roles: boolean, };

/**
 * How many records were deleted.
 */
export type DeleteResponse = { deleted: number, };

/**
 * The body of an error response.
 */
export type ErrorBody = { error: string,
/**
 * The submitted fields that were rejected, each with why.
 */
fields?: { [key in string]: Array<string> }, };

/**
 * How a field is rendered and edited in the UI.
 */
export type FieldKind = "string" | "text" | "integer" | "float" | "decimal" | "boolean" | "date" | "time" | "date_time" | "uuid" | "json" | "enum" | "binary" | "array" | "other";

/**
 * A field of a model as exposed to the UI.
 */
export type FieldMeta = { name: string, label: string, kind: FieldKind, nullable: boolean, primary_key: boolean, auto_increment: boolean, readonly: boolean, unique: boolean,
/**
 * Whether a date-time carries a time zone.
 */
timezone: boolean, max_length?: number, choices?: Array<string>, relation?: RelationMeta,
/**
 * Whether this is the users' role column, whose values are picked from the roles.
 */
role?: boolean, };

/**
 * A login form that posts JSON credentials to an endpoint and reads a bearer
 * token from the JSON response.
 */
export type FormLogin = {
/**
 * URL the credentials are posted to.
 */
endpoint: string,
/**
 * JSON key of the identifier (for example `email`).
 */
identifier_field: string,
/**
 * Label of the identifier input.
 */
identifier_label: string,
/**
 * JSON key of the password.
 */
password_field: string,
/**
 * JSON key holding the token in the response.
 */
token_field: string, };

/**
 * What a role may do with one model. Any other permission implies `view`.
 */
export type Grant = { view: boolean, create: boolean, update: boolean, delete: boolean, export: boolean,
/**
 * Names of the custom actions the role may run.
 */
actions: Array<string>, };

export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]: JsonValue } | null;

/**
 * The records a bulk delete or custom action applies to.
 */
export type KeysBody = { keys: Array<string>, };

/**
 * A page of records, with the labels of the records they point at.
 */
export type ListResponse = { labels: { [key in string]: { [key in string]: string } }, records: Array<AdminRecord>, total: number, page: number, per_page: number, pages: number, };

/**
 * How the UI signs users in.
 */
export type LoginMethod = { "type": "form" } & FormLogin | { "type": "redirect", url: string, } | { "type": "none" };

/**
 * Options for a foreign-key picker.
 */
export type LookupResponse = { choices: Array<Choice>, };

/**
 * Everything the UI needs to render a model.
 */
export type ModelMeta = { name: string, label: string, table: string, primary_key: Array<string>, fields: Array<FieldMeta>, list_display: Array<string>, search_fields: Array<string>, list_filter: Array<string>, ordering: Ordering, per_page: number, display_field: string | null, actions: Array<ActionMeta>, related: Array<RelatedMeta>, capabilities: Capabilities, };

/**
 * Default ordering of a list.
 */
export type Ordering = { field: string, direction: SortDirection, };

/**
 * A record, with the labels of the records it points at.
 */
export type RecordResponse = { record: AdminRecord, labels: { [key in string]: { [key in string]: string } }, };

/**
 * Another model whose foreign key points at this one.
 */
export type RelatedMeta = { model: string, field: string, label: string, };

/**
 * A foreign key from this field to another table.
 */
export type RelationMeta = {
/**
 * The referenced table.
 */
table: string,
/**
 * The referenced column.
 */
column: string,
/**
 * The registered admin model for `table`, if any.
 */
model: string | null, };

/**
 * A role as submitted by the UI. Fields left out are kept on update.
 */
export type RoleInput = { name?: string, permissions?: { [key in string]: Grant }, };

/**
 * A role by id and name.
 */
export type RoleRef = { id: number, name: string, };

/**
 * One role.
 */
export type RoleResponse = { role: RoleWithUsers, };

/**
 * A role and how many users have it.
 */
export type RoleWithUsers = {
/**
 * How many users have the role.
 */
users: number, id: number, name: string, permissions: { [key in string]: Grant }, };

/**
 * All roles.
 */
export type RolesResponse = { roles: Array<RoleWithUsers>, };

/**
 * The models the user may view.
 */
export type SchemaResponse = { models: Array<ModelMeta>, };

/**
 * Sort direction.
 */
export type SortDirection = "asc" | "desc";

/**
 * Model name to its number of records, for the models the user may view.
 */
export type StatsResponse = { counts: { [key in string]: number }, };
