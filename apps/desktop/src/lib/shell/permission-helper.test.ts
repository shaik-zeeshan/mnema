// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, test } from "bun:test";
import { permissionHelperGranted, permissionHelperKind } from "./permission-helper";

const perms = (screen, microphone = "denied", systemAudio = "unknown") => ({ screen, microphone, systemAudio });

describe("permission helper", () => {
  test("unknown or missing kind falls back to screen", () => {
    expect(permissionHelperKind(null)).toBe("screen");
    expect(permissionHelperKind("toString")).toBe("screen");
    expect(permissionHelperKind("mic")).toBe("screen");
    expect(permissionHelperKind("systemAudio")).toBe("systemAudio");
  });

  test("granted only on the kind's own switch", () => {
    expect(permissionHelperGranted("screen", perms("granted"), false)).toBe(true);
    expect(permissionHelperGranted("screen", perms("denied", "granted"), false)).toBe(false);
    expect(permissionHelperGranted("microphone", perms("denied", "granted"), false)).toBe(true);
    expect(permissionHelperGranted("screen", null, false)).toBe(false);
  });

  test("system audio waits until a tap has heard sound", () => {
    expect(permissionHelperGranted("systemAudio", perms("granted", "granted", "unknown"), false)).toBe(false);
    expect(permissionHelperGranted("systemAudio", perms("granted", "granted", "possibly_blocked"), false)).toBe(false);
    expect(permissionHelperGranted("systemAudio", perms("granted", "granted", "assumed_working"), false)).toBe(true);
  });

  test("accessibility reads the trust flag, not capture permissions", () => {
    expect(permissionHelperGranted("accessibility", perms("granted", "granted"), false)).toBe(false);
    expect(permissionHelperGranted("accessibility", null, true)).toBe(true);
  });
});
