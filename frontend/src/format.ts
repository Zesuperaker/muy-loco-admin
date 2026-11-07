import { parseJson } from "./json";
import type { FieldMeta, JsonValue } from "./types";

const pad = (n: number) => String(n).padStart(2, "0");

/** `2024-01-31T10:30:00+02:00` → `2024-01-31T09:30:00` in local time, for datetime-local inputs. */
export function toLocalInput(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  );
}

/** Value of a field as shown in an input. */
export function toInput(field: FieldMeta, value: JsonValue | undefined): string {
  if (value === null || value === undefined) return "";
  switch (field.kind) {
    case "json":
    case "array":
      return JSON.stringify(value, null, 2);
    case "date_time":
      if (typeof value !== "string") return String(value);
      return field.timezone ? toLocalInput(value) : value.replace(" ", "T").replace(/\.\d+$/, "");
    case "time":
      return String(value).replace(/\.\d+$/, "");
    default:
      return typeof value === "string" ? value : String(value);
  }
}

export class InputError extends Error {}

/** Convert an input's text into the JSON sent to the API. */
export function fromInput(field: FieldMeta, text: string): JsonValue {
  const trimmed = text.trim();
  if (field.kind !== "string" && field.kind !== "text") {
    if (trimmed === "") return null;
  } else if (text === "" && field.nullable) {
    return null;
  }
  switch (field.kind) {
    case "integer": {
      if (!/^-?\d+$/.test(trimmed)) throw new InputError("must be a whole number");
      const n = Number(trimmed);
      // Integers JavaScript would round go as text, which the API accepts.
      return Number.isSafeInteger(n) ? n : trimmed;
    }
    case "float": {
      const n = Number(trimmed);
      if (!Number.isFinite(n)) throw new InputError("must be a number");
      return n;
    }
    case "decimal":
      if (!/^-?\d+(\.\d+)?$/.test(trimmed)) throw new InputError("must be a decimal number");
      return trimmed;
    case "boolean":
      return trimmed === "true";
    case "json":
      try {
        return parseJson(trimmed) as JsonValue;
      } catch {
        throw new InputError("must be valid JSON");
      }
    case "date_time": {
      const withSeconds = /T\d{2}:\d{2}$/.test(trimmed) ? `${trimmed}:00` : trimmed;
      if (!field.timezone) return withSeconds;
      const date = new Date(withSeconds);
      if (Number.isNaN(date.getTime())) throw new InputError("must be a date and time");
      return date.toISOString();
    }
    default:
      return text;
  }
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/** The whole value as text, for a read-only field. */
export function displayFull(field: FieldMeta | undefined, value: JsonValue | undefined): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "object") return JSON.stringify(value, null, 2);
  const text = String(value);
  if (field?.kind === "date_time") {
    if (field.timezone) {
      const date = new Date(text);
      return Number.isNaN(date.getTime()) ? text : date.toLocaleString();
    }
    return text.replace("T", " ").replace(/\.\d+$/, "");
  }
  return text;
}

/** Short text for a table cell. */
export function display(field: FieldMeta | undefined, value: JsonValue | undefined): string {
  if (value !== null && typeof value === "object") return truncate(JSON.stringify(value), 60);
  return truncate(displayFull(field, value), 80);
}

/**
 * A record key part as the server encodes it (`encode_key`): `%`, `,`,
 * control and non-ASCII characters percent-encoded.
 */
export function encodeKeyPart(text: string): string {
  let out = "";
  for (const byte of new TextEncoder().encode(text)) {
    out +=
      byte < 0x20 || byte >= 0x7f || byte === 0x25 || byte === 0x2c
        ? `%${byte.toString(16).toUpperCase().padStart(2, "0")}`
        : String.fromCharCode(byte);
  }
  return out;
}

/** Text of a value used as a key in label maps and filters. */
export function keyText(value: JsonValue | undefined): string {
  if (value === null || value === undefined) return "";
  return typeof value === "string" ? value : JSON.stringify(value);
}
