// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { GENERIC_ERROR_MESSAGE, humanizeError } from "./format-error";

describe("humanizeError", () => {
  it("returns plain strings, capitalized and trimmed", () => {
    expect(humanizeError("disk is full")).toBe("Disk is full");
    expect(humanizeError("  network timeout  ")).toBe("Network timeout");
  });

  it("uses the message of Error instances", () => {
    expect(humanizeError(new Error("boom"))).toBe("Boom");
  });

  it("extracts message-like fields from objects (no raw JSON)", () => {
    expect(humanizeError({ message: "permission denied" })).toBe("Permission denied");
    expect(humanizeError({ error: "model not found" })).toBe("Model not found");
    expect(humanizeError({ reason: "no provider configured" })).toBe("No provider configured");
    expect(humanizeError({ code: 500, detail: "upstream failed" })).toBe("Upstream failed");
  });

  it("parses JSON-encoded error strings instead of showing the JSON", () => {
    expect(humanizeError('{"message":"capture failed"}')).toBe("Capture failed");
    expect(humanizeError('{"Io":"file not found"}')).toBe("Io: file not found");
  });

  it("humanizes serde externally-tagged enum variants", () => {
    expect(humanizeError({ PermissionDenied: "screen recording" })).toBe(
      "Permission denied: screen recording",
    );
    expect(humanizeError({ DisplayUnavailable: null })).toBe("Display unavailable");
  });

  it("strips a redundant leading Error: prefix", () => {
    expect(humanizeError("Error: something broke")).toBe("Something broke");
  });

  it("falls back instead of dumping JSON for unreadable shapes", () => {
    expect(humanizeError({ a: 1, b: 2 })).toBe(GENERIC_ERROR_MESSAGE);
    expect(humanizeError(null)).toBe(GENERIC_ERROR_MESSAGE);
    expect(humanizeError(undefined)).toBe(GENERIC_ERROR_MESSAGE);
    expect(humanizeError("")).toBe(GENERIC_ERROR_MESSAGE);
    expect(humanizeError({}, "Couldn't load.")).toBe("Couldn't load.");
  });

  it("falls back instead of leaking raw JSON-encoded strings with no readable message", () => {
    expect(humanizeError('{"a":1,"b":2}')).toBe(GENERIC_ERROR_MESSAGE);
    expect(humanizeError('[{"a":1,"b":2}]')).toBe(GENERIC_ERROR_MESSAGE);
  });

  it("truncates very long messages", () => {
    const long = "x".repeat(500);
    const out = humanizeError(long);
    expect(out.length).toBeLessThanOrEqual(300);
    expect(out.endsWith("…")).toBe(true);
  });
});
