// Pins the permission tri-state display contract in onboarding-attention.ts,
// especially system audio's inferred states (ADR 0052): `assumed_working` and
// `possibly_blocked` have no OS-readable grant, so their action/label/tone
// mapping is the only surface a regression would show up on.
import { describe, expect, test } from "bun:test";
import {
  permissionActionFor,
  permissionLabelFor,
  permissionToneFor,
} from "../src/routes/onboarding/onboarding-attention";

describe("permissionActionFor", () => {
  test("granted / unsupported / assumed_working -> no action", () => {
    expect(permissionActionFor("granted")).toBeNull();
    expect(permissionActionFor("unsupported")).toBeNull();
    // ADR 0052: a tap that has delivered sound proved the grant — no button.
    expect(permissionActionFor("assumed_working")).toBeNull();
  });

  test("denied / restricted -> settings deep-link (macOS never re-prompts)", () => {
    expect(permissionActionFor("denied")).toEqual({ label: "Open Settings", mode: "settings" });
    expect(permissionActionFor("restricted")).toEqual({ label: "Open Settings", mode: "settings" });
  });

  test("not_determined -> request action", () => {
    expect(permissionActionFor("not_determined")).toEqual({ label: "Grant access", mode: "request" });
  });

  test("possibly_blocked -> request action (system audio has no 'denied')", () => {
    expect(permissionActionFor("possibly_blocked")).toEqual({ label: "Grant access", mode: "request" });
  });
});

describe("permissionLabelFor", () => {
  test("assumed_working -> Working", () => {
    expect(permissionLabelFor("assumed_working")).toBe("Working");
  });
  test("possibly_blocked -> May be blocked", () => {
    expect(permissionLabelFor("possibly_blocked")).toBe("May be blocked");
  });
  test("not_determined -> Not requested", () => {
    expect(permissionLabelFor("not_determined")).toBe("Not requested");
  });
});

describe("permissionToneFor", () => {
  test("granted and assumed_working -> ok", () => {
    expect(permissionToneFor("granted")).toBe("ok");
    expect(permissionToneFor("assumed_working")).toBe("ok");
  });
  test("not_determined -> pending", () => {
    expect(permissionToneFor("not_determined")).toBe("pending");
  });
  test("possibly_blocked -> blocked", () => {
    expect(permissionToneFor("possibly_blocked")).toBe("blocked");
  });
});
