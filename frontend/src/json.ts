type RawJson = (text: string) => unknown;

/** `JSON.rawJSON`, where the browser has it (not yet in TypeScript's lib). */
const rawJSON = (JSON as unknown as { rawJSON?: RawJson }).rawJSON;

/**
 * `JSON.parse`, keeping integers that JavaScript would round exactly as
 * written: they become `JSON.rawJSON` values, which `JSON.stringify` writes
 * back unchanged. The API sends such integers in plain columns as strings;
 * this covers the ones inside JSON columns. Browsers without `JSON.rawJSON`
 * parse as usual.
 */
export function parseJson(text: string): unknown {
  if (!rawJSON) return JSON.parse(text);
  const keep = rawJSON;
  return JSON.parse(text, (_key, value: unknown, context?: { source?: string }) =>
    typeof value === "number" &&
    !Number.isSafeInteger(value) &&
    context?.source !== undefined &&
    /^-?\d+$/.test(context.source)
      ? keep(context.source)
      : value,
  );
}
