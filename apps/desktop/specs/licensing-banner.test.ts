// Banner policy (licensing-banner.ts): thresholds, tone ramp, precedence, and
// dismissal keying for the app-shell licensing banner.

import { describe, expect, test } from "bun:test";
import { bannerFor, bannerVisible, days } from "../src/lib/licensing-banner";
import {
  LICENSED_LAPSED,
  READ_ONLY,
  REVOKED,
  TRIAL_NOT_STARTED,
  licensed,
  licensedOverCap,
  licensedPending,
  trial,
} from "./fixtures/license-status";

describe("bannerFor: trial thresholds + tone ramp", () => {
  test("8 days left → no banner (final week only)", () => {
    expect(bannerFor(trial(8))).toBeNull();
  });

  test("7 days → info", () => {
    const banner = bannerFor(trial(7));
    expect(banner).toEqual({
      kind: "trial",
      daysLeft: 7,
      tone: "info",
      message: expect.stringContaining("Free trial ends in 7 days."),
      dismissKey: 7,
    });
  });

  test("3 days → warn", () => {
    const banner = bannerFor(trial(3));
    expect(banner?.kind).toBe("trial");
    if (banner?.kind === "trial") {
      expect(banner.tone).toBe("warn");
      expect(banner.message).toContain("Free trial ends in 3 days.");
    }
  });

  test("1 day → urgent + 'ends today'", () => {
    const banner = bannerFor(trial(1));
    expect(banner?.kind).toBe("trial");
    if (banner?.kind === "trial") {
      expect(banner.tone).toBe("urgent");
      expect(banner.message).toContain("Your free trial ends today.");
    }
  });

  test("healthy states → no banner", () => {
    expect(bannerFor(null)).toBeNull();
    expect(bannerFor(TRIAL_NOT_STARTED)).toBeNull();
    expect(bannerFor(licensed())).toBeNull();
    // Over-cap is deliberately Settings-only.
    expect(bannerFor(licensedOverCap())).toBeNull();
  });
});

describe("bannerFor: firm (non-dismissible) states", () => {
  test("readOnly / revoked / lapsed activation have a null dismissKey", () => {
    expect(bannerFor(READ_ONLY)).toEqual({ kind: "readOnly", dismissKey: null });
    expect(bannerFor(REVOKED)).toEqual({ kind: "revoked", dismissKey: null });
    expect(bannerFor(LICENSED_LAPSED)).toEqual({ kind: "lapsed", dismissKey: null });
  });

  test("firm banners are visible regardless of any dismissed key", () => {
    expect(bannerVisible(bannerFor(READ_ONLY), 3)).toBe(true);
    expect(bannerVisible(bannerFor(REVOKED), null)).toBe(true);
    expect(bannerVisible(bannerFor(LICENSED_LAPSED), 1)).toBe(true);
  });
});

describe("bannerFor: provisional activation nudge", () => {
  test("pending ≤3 days → nudge with day-keyed dismissal", () => {
    expect(bannerFor(licensedPending(3))).toEqual({
      kind: "provisional",
      daysLeft: 3,
      dismissKey: 3,
    });
    expect(bannerFor(licensedPending(1))).toEqual({
      kind: "provisional",
      daysLeft: 1,
      dismissKey: 1,
    });
  });

  test("pending >3 days → no banner yet", () => {
    expect(bannerFor(licensedPending(4))).toBeNull();
    expect(bannerFor(licensedPending(7))).toBeNull();
  });
});

describe("precedence", () => {
  test("lapsed activation outranks the provisional nudge shape", () => {
    // A lapsed status is firm even though it's also `licensed`.
    expect(bannerFor(LICENSED_LAPSED)?.kind).toBe("lapsed");
  });

  test("readOnly and revoked are their own banners, not trial copy", () => {
    expect(bannerFor(READ_ONLY)?.kind).toBe("readOnly");
    expect(bannerFor(REVOKED)?.kind).toBe("revoked");
  });
});

describe("dismissal keying", () => {
  test("dismissing at the current day-count hides the banner", () => {
    const banner = bannerFor(trial(5));
    expect(bannerVisible(banner, 5)).toBe(false);
  });

  test("a dismissed banner re-surfaces when the day count drops", () => {
    // Dismissed at 3 days; next recompute says 2 → must re-surface.
    expect(bannerVisible(bannerFor(trial(2)), 3)).toBe(true);
    expect(bannerVisible(bannerFor(licensedPending(2)), 3)).toBe(true);
  });

  test("no banner → not visible", () => {
    expect(bannerVisible(null, null)).toBe(false);
  });
});

describe("days()", () => {
  test("singular/plural", () => {
    expect(days(1)).toBe("1 day");
    expect(days(2)).toBe("2 days");
  });
});
