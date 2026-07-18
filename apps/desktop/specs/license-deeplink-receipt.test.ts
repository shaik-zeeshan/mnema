// Deep-link receipt face policy (license-deeplink-receipt.ts): which
// license_status result maps to which face of the LicenseDeepLinkModal.

import { describe, expect, test } from "bun:test";
import { receiptFaceFor } from "../src/lib/license-deeplink-receipt";
import {
  LICENSED_LAPSED,
  READ_ONLY,
  TRIAL_NOT_STARTED,
  licensed,
  licensedOverCap,
  licensedPending,
  trial,
} from "./fixtures/license-status";

describe("receiptFaceFor — activate/claim flows", () => {
  test("non-licensed statuses keep the working face (result still in flight)", () => {
    for (const status of [null, TRIAL_NOT_STARTED, trial(7), READ_ONLY]) {
      expect(receiptFaceFor("activate", null, status)).toEqual({ face: "working" });
      expect(receiptFaceFor("claim", null, status)).toEqual({ face: "working" });
    }
  });

  test("activated → happy receipt with owner name and window date", () => {
    const status = licensed();
    expect(receiptFaceFor("claim", trial(3), status)).toEqual({
      face: "activated",
      owner: "Ada Lovelace",
      updateThroughMs: 1_731_536_000_000,
    });
  });

  test("owner falls back to email when the key carries no name", () => {
    const face = receiptFaceFor("activate", null, licensed({ name: "" }));
    expect(face).toMatchObject({ face: "activated", owner: "owner@example.com" });
  });

  test("pending → provisional face with days-to-connect", () => {
    expect(receiptFaceFor("claim", null, licensedPending(5))).toEqual({
      face: "pending",
      owner: "Ada Lovelace",
      provisionalDaysLeft: 5,
    });
  });

  test("refusedOverCap → actionable failure with the server's links", () => {
    expect(receiptFaceFor("activate", trial(2), licensedOverCap())).toEqual({
      face: "overCap",
      resetUrl: "https://license.example/reset",
      buyUrl: "https://mnema.day/#pricing",
    });
  });

  test("lapsed stays working — no invented face seconds after a deep link", () => {
    expect(receiptFaceFor("activate", null, LICENSED_LAPSED)).toEqual({ face: "working" });
  });
});

describe("receiptFaceFor — renewed flow", () => {
  const before = licensed({ updateThroughMs: 1_700_000_000_000 });
  const after = licensed({ updateThroughMs: 1_731_536_000_000 });

  test("unchanged through-date is a pre-extension emit, not the result", () => {
    expect(receiptFaceFor("renewed", before, before)).toEqual({ face: "working" });
  });

  test("extended window → renewed face with the old date for the strikethrough", () => {
    expect(receiptFaceFor("renewed", before, after)).toEqual({
      face: "renewed",
      updateThroughMs: 1_731_536_000_000,
      wasMs: 1_700_000_000_000,
    });
  });

  test("no licensed baseline (cold window) → renewed without a was-date", () => {
    expect(receiptFaceFor("renewed", trial(1), after)).toEqual({
      face: "renewed",
      updateThroughMs: 1_731_536_000_000,
      wasMs: null,
    });
  });

  test("over-cap outranks the renewal face", () => {
    expect(receiptFaceFor("renewed", before, licensedOverCap())).toMatchObject({
      face: "overCap",
    });
  });
});
