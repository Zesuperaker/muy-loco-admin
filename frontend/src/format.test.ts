import { describe, expect, it } from "vitest";
import { InputError, display, fromInput, keyText, toInput, toLocalInput } from "./format";
import { field } from "./test/utils";
import { paths } from "./paths";

describe("toInput", () => {
  it("renders values for inputs", () => {
    expect(toInput(field("a"), null)).toBe("");
    expect(toInput(field("a"), undefined)).toBe("");
    expect(toInput(field("a"), "x")).toBe("x");
    expect(toInput(field("n", { kind: "integer" }), 5)).toBe("5");
    expect(toInput(field("b", { kind: "boolean" }), true)).toBe("true");
    expect(toInput(field("j", { kind: "json" }), { a: 1 })).toBe('{\n  "a": 1\n}');
    expect(toInput(field("t", { kind: "time" }), "10:00:00.123")).toBe("10:00:00");
    expect(toInput(field("d", { kind: "date_time" }), "2024-01-01T10:00:00.5")).toBe(
      "2024-01-01T10:00:00",
    );
    expect(toInput(field("d", { kind: "date_time" }), "2024-01-01 10:00:00")).toBe(
      "2024-01-01T10:00:00",
    );
    expect(toInput(field("d", { kind: "date_time" }), 5)).toBe("5");
    const tz = toInput(field("d", { kind: "date_time", timezone: true }), "2024-01-01T10:00:00Z");
    expect(tz).toBe(toLocalInput("2024-01-01T10:00:00Z"));
  });

  it("formats local date-times", () => {
    expect(toLocalInput("not a date")).toBe("");
    expect(toLocalInput("2024-06-01T12:30:45")).toBe("2024-06-01T12:30:45");
  });
});

describe("fromInput", () => {
  it("converts text to typed JSON", () => {
    expect(fromInput(field("n", { kind: "integer" }), " 42 ")).toBe(42);
    expect(fromInput(field("f", { kind: "float" }), "1.5")).toBe(1.5);
    expect(fromInput(field("d", { kind: "decimal" }), "10.50")).toBe("10.50");
    expect(fromInput(field("b", { kind: "boolean" }), "true")).toBe(true);
    expect(fromInput(field("b", { kind: "boolean" }), "false")).toBe(false);
    expect(fromInput(field("j", { kind: "json" }), '{"a":[1]}')).toEqual({ a: [1] });
    expect(fromInput(field("s"), "  keep spaces ")).toBe("  keep spaces ");
    expect(fromInput(field("s"), "")).toBe("");
    expect(fromInput(field("s", { nullable: true }), "")).toBeNull();
    expect(fromInput(field("t", { kind: "text", nullable: true }), "")).toBeNull();
    expect(fromInput(field("n", { kind: "integer" }), "")).toBeNull();
    expect(fromInput(field("e", { kind: "enum" }), "draft")).toBe("draft");
  });

  it("handles date-times", () => {
    expect(fromInput(field("d", { kind: "date_time" }), "2024-01-01T10:00")).toBe(
      "2024-01-01T10:00:00",
    );
    const tz = fromInput(field("d", { kind: "date_time", timezone: true }), "2024-01-01T10:00");
    expect(tz).toBe(new Date("2024-01-01T10:00:00").toISOString());
  });

  it("rejects invalid input", () => {
    expect(() => fromInput(field("n", { kind: "integer" }), "1.5")).toThrow(InputError);
    expect(() => fromInput(field("f", { kind: "float" }), "abc")).toThrow("must be a number");
    expect(() => fromInput(field("d", { kind: "decimal" }), "1e3")).toThrow("decimal");
    expect(() => fromInput(field("j", { kind: "json" }), "{")).toThrow("valid JSON");
    expect(() =>
      fromInput(field("d", { kind: "date_time", timezone: true }), "garbage"),
    ).toThrow("date and time");
  });
});

describe("display", () => {
  it("formats cells", () => {
    expect(display(undefined, null)).toBe("—");
    expect(display(undefined, undefined)).toBe("—");
    expect(display(undefined, true)).toBe("Yes");
    expect(display(undefined, false)).toBe("No");
    expect(display(undefined, { a: 1 })).toBe('{"a":1}');
    expect(display(undefined, "x".repeat(100))).toHaveLength(80);
    expect(display(undefined, 12)).toBe("12");
    expect(display(field("d", { kind: "date_time" }), "2024-01-01T10:00:00.25")).toBe(
      "2024-01-01 10:00:00",
    );
    expect(display(field("d", { kind: "date_time", timezone: true }), "bad")).toBe("bad");
    expect(display(field("d", { kind: "date_time", timezone: true }), "2024-01-01T10:00:00Z")).toBe(
      new Date("2024-01-01T10:00:00Z").toLocaleString(),
    );
  });

  it("renders key text", () => {
    expect(keyText(null)).toBe("");
    expect(keyText(undefined)).toBe("");
    expect(keyText("a")).toBe("a");
    expect(keyText(5)).toBe("5");
    expect(keyText(true)).toBe("true");
  });
});

describe("paths", () => {
  it("encodes segments", () => {
    expect(paths.dashboard()).toBe("/");
    expect(paths.list("post tags", "?q=1")).toBe("/models/post%20tags?q=1");
    expect(paths.create("posts")).toBe("/models/posts/new");
    expect(paths.record("post_tags", "1,a%2Cb")).toBe("/models/post_tags/records/1%2Ca%252Cb");
  });
});
