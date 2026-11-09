export type FieldKind =
  | "string"
  | "text"
  | "integer"
  | "float"
  | "decimal"
  | "boolean"
  | "date"
  | "time"
  | "date_time"
  | "uuid"
  | "json"
  | "enum"
  | "binary"
  | "array"
  | "other";

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface RelationMeta {
  table: string;
  column: string;
  model: string | null;
}

export interface FieldMeta {
  name: string;
  label: string;
  kind: FieldKind;
  nullable: boolean;
  primary_key: boolean;
  auto_increment: boolean;
  readonly: boolean;
  unique: boolean;
  timezone: boolean;
  max_length?: number;
  choices?: string[];
  relation?: RelationMeta;
  /** The users' role column: picked from the roles. */
  role?: boolean;
}

export interface Capabilities {
  view: boolean;
  create: boolean;
  update: boolean;
  delete: boolean;
  export: boolean;
}

export interface ModelMeta {
  name: string;
  label: string;
  table: string;
  primary_key: string[];
  fields: FieldMeta[];
  list_display: string[];
  search_fields: string[];
  list_filter: string[];
  ordering: { field: string; direction: "asc" | "desc" };
  per_page: number;
  display_field: string | null;
  actions: { name: string; label: string }[];
  related: { model: string; field: string; label: string }[];
  capabilities: Capabilities;
}

export interface FormLogin {
  type: "form";
  endpoint: string;
  identifier_field: string;
  identifier_label: string;
  password_field: string;
  token_field: string;
}

export type LoginMethod =
  | FormLogin
  | { type: "redirect"; url: string }
  | { type: "none" };

export interface AdminConfig {
  title: string;
  base_path: string;
  /** Target of the "Visit site" link; `null` hides it. */
  site_url: string | null;
  login: LoginMethod;
}

export interface AdminUser {
  id: string;
  name: string;
  /** The user's role; `null` for full access. */
  role?: { id: number; name: string } | null;
  /** Roles are on and the user has full access. */
  can_manage_roles?: boolean;
}

/** What a role may do with one model. Any other permission implies `view`. */
export interface Grant {
  view: boolean;
  create: boolean;
  update: boolean;
  delete: boolean;
  export: boolean;
  actions: string[];
}

/** Model name to what the role may do with it; models not listed are hidden. */
export type Permissions = Record<string, Grant>;

export interface Role {
  id: number;
  name: string;
  permissions: Permissions;
  /** How many users have the role. */
  users: number;
}

export interface AdminRecord {
  key: string;
  label: string;
  values: Record<string, JsonValue>;
}

/** Foreign-key field name, then the key value as text, to the target label. */
export type Labels = Record<string, Record<string, string>>;

/**
 * How many records a list has: exactly, more than `value` (counting stops there), or about
 * `value` (Postgres's estimate for a whole table).
 */
export interface Count {
  kind: "exact" | "more_than" | "estimate";
  value: number;
}

export interface Page {
  records: AdminRecord[];
  per_page: number;
  /** The `after` cursor of the next page. */
  next: string | null;
  /** The `before` cursor of the previous page. */
  prev: string | null;
  labels: Labels;
}

export interface Choice {
  key: string;
  value: JsonValue;
  label: string;
}
