import { afterEach, describe, expect, it, vi } from "vitest";
import {
  countNote,
  formatCount,
  InputError,
  display,
  displayFull,
  encodeKeyPart,
  fromInput,
  keyText,
  toInput,
  toLocalInput,
} from "./format";
import { parseJson } from "./json";
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

/** Make the browser's language `language`, until the mocks are restored. */
const browserIn = (language: string) =>
  vi.spyOn(navigator, "language", "get").mockReturnValue(language);

describe("float input", () => {
  const float = field("f", { kind: "float" });
  afterEach(() => vi.restoreAllMocks());

  it("reads a decimal comma where the browser's language writes one", () => {
    browserIn("de-DE");
    expect(fromInput(float, "1,5")).toBe(1.5);
    expect(fromInput(float, "-0,25")).toBe(-0.25);
    // A value is shown with a point, so that is read too.
    expect(fromInput(float, "1.5")).toBe(1.5);
    expect(fromInput(float, "1e3")).toBe(1000);
    // More than one separator is refused rather than guessed at.
    for (const text of ["1.234,5", "1,234.5", "1,234,5", "1,5,", ","]) {
      expect(() => fromInput(float, text), text).toThrow("must be a number");
    }
    // One separator is always the decimal one, as in a number input there:
    // thousands cannot be grouped, with a comma or a point.
    expect(fromInput(float, "1,234")).toBe(1.234);
    expect(fromInput(float, "1.234")).toBe(1.234);
  });

  it("refuses a comma elsewhere, where it may group thousands", () => {
    // `en_US` is not a language tag `Intl` reads; it must not stop the save.
    for (const language of ["en-US", "en_US"]) {
      browserIn(language);
      expect(fromInput(float, "1.5")).toBe(1.5);
      for (const text of ["1,5", "1,234", "1,234.5"]) {
        expect(() => fromInput(float, text), text).toThrow("must be a number");
      }
    }
  });

  it("refuses what is not a decimal number", () => {
    for (const text of ["1e", "0x10", "Infinity", "NaN", "1.2.3", "--1", "1 000"]) {
      expect(() => fromInput(float, text), text).toThrow("must be a number");
    }
    expect(fromInput(float, " .5 ")).toBe(0.5);
    expect(fromInput(float, "+2.")).toBe(2);
    expect(fromInput(field("f", { kind: "float", nullable: true }), "")).toBeNull();
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

describe("formatCount", () => {
  it("formats exact, capped and estimated counts", () => {
    expect(formatCount({ kind: "exact", value: 1234 })).toBe("1,234");
    expect(countNote({ kind: "exact", value: 1234 })).toBeUndefined();
    expect(formatCount({ kind: "more_than", value: 10000 })).toBe("10,000+");
    expect(formatCount({ kind: "estimate", value: 1234567 })).toBe("~1.2M");
    expect(countNote({ kind: "estimate", value: 1234567 })).toBe(
      "Estimated from the database's statistics",
    );
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

describe("pre-release fixes", () => {
  it("sends integers JavaScript would round as text", () => {
    const f = field("views", { kind: "integer" });
    expect(fromInput(f, "42")).toBe(42);
    expect(fromInput(f, " -9007199254740991 ")).toBe(-9007199254740991);
    expect(fromInput(f, "9007199254740993")).toBe("9007199254740993");
    expect(() => fromInput(f, "1e3")).toThrow("must be a whole number");
  });

  it("keeps big integers in JSON exactly", () => {
    const value = parseJson('{"n":9007199254740993,"small":1,"float":1.5,"list":[12345678901234567890]}');
    expect(JSON.stringify(value)).toBe('{"n":9007199254740993,"small":1,"float":1.5,"list":[12345678901234567890]}');
    const f = field("meta", { kind: "json" });
    expect(JSON.stringify(fromInput(f, '{"n": 9007199254740993}'))).toBe('{"n":9007199254740993}');
  });

  it("shows whole values for read-only fields", () => {
    const text = field("body", { kind: "text" });
    expect(displayFull(text, "y".repeat(100))).toBe("y".repeat(100));
    expect(display(text, "y".repeat(100))).toHaveLength(80);
    expect(displayFull(field("meta", { kind: "json" }), { a: 1 })).toBe('{\n  "a": 1\n}');
    expect(displayFull(text, null)).toBe("—");
    expect(displayFull(field("flag", { kind: "boolean" }), true)).toBe("Yes");
  });

  it("encodes key parts like the server", () => {
    expect(encodeKeyPart("plain-1_a.b")).toBe("plain-1_a.b");
    expect(encodeKeyPart("a,b%c")).toBe("a%2Cb%25c");
    expect(encodeKeyPart("é\n")).toBe("%C3%A9%0A");
    expect(encodeKeyPart("a/b ?")).toBe("a/b ?");
  });
});
