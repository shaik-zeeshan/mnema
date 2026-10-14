// License panel policy (licensing-panel.ts): the money path (buy vs renew),
// badge, external-URL vetting, and the onboarding status line — plus the
// purchase/renewal checkout-link split in licensing.ts.

import { describe, expect, test } from "bun:test";
import {
  badgeFor,
  checkoutUrlFor,
  fmtLicenseDate,
  licensedOutOfWindow,
  safeExternalUrl,
  showBuyFor,
  statusHintFor,
  statusInlineFor,
  statusLineFor,
} from "../src/lib/licensing-panel";
import { LICENSE_CHECKOUT_URL, RENEWAL_CHECKOUT_URL } from "../src/lib/licensing";
import {
  ALL_VARIANTS,
  LICENSED_LAPSED,
  READ_ONLY,
  REVOKED,
  TRIAL_NOT_STARTED,
  licensed,
  licensedOverCap,
  licensedPending,
  trial,
} from "./fixtures/license-status";

describe("money path: checkoutUrlFor / showBuyFor", () => {
  test("licensed + inWindow → license checkout URL, Buy row hidden", () => {
    const status = licensed({ inWindow: true });
    expect(checkoutUrlFor(status)).toBe(LICENSE_CHECKOUT_URL);
    expect(showBuyFor(status)).toBe(false);
    expect(licensedOutOfWindow(status)).toBe(false);
  });

  test("licensed + !inWindow → RENEWAL checkout URL, Renew row shown", () => {
    const status = licensed({ inWindow: false });
    expect(checkoutUrlFor(status)).toBe(RENEWAL_CHECKOUT_URL);
    expect(showBuyFor(status)).toBe(true);
    expect(licensedOutOfWindow(status)).toBe(true);
  });

  test("everyone else buys the license", () => {
    for (const status of [null, TRIAL_NOT_STARTED, trial(10), READ_ONLY, REVOKED]) {
      expect(checkoutUrlFor(status)).toBe(LICENSE_CHECKOUT_URL);
      expect(showBuyFor(status)).toBe(true);
    }
  });

  test("renewal and purchase are distinct links (distinct success URLs)", () => {
    expect(RENEWAL_CHECKOUT_URL).not.toBe(LICENSE_CHECKOUT_URL);
  });
});

describe("badgeFor", () => {
  test("per-variant labels", () => {
    expect(badgeFor(licensed())).toEqual({ label: "licensed", variant: "accent" });
    expect(badgeFor(licensedPending(5))).toEqual({ label: "activating…", variant: "plain" });
    expect(badgeFor(licensedOverCap())).toEqual({ label: "device limit", variant: "warn" });
    expect(badgeFor(LICENSED_LAPSED)).toEqual({ label: "not activated", variant: "warn" });
    expect(badgeFor(trial(3))).toEqual({ label: "trial", variant: "info" });
    expect(badgeFor(TRIAL_NOT_STARTED)).toEqual({ label: "trial ready", variant: "plain" });
    expect(badgeFor(READ_ONLY)).toEqual({ label: "read-only", variant: "warn" });
    expect(badgeFor(REVOKED)).toEqual({ label: "revoked", variant: "warn" });
    expect(badgeFor(null)).toBeNull();
  });

  test("every wire variant produces a badge", () => {
    for (const status of ALL_VARIANTS) {
      expect(badgeFor(status)).not.toBeNull();
    }
  });
});

describe("statusHintFor / statusInlineFor (Settings Status row)", () => {
  const through = fmtLicenseDate(1_731_536_000_000);

  test("activated in-window owner: one composite line", () => {
    expect(statusHintFor(licensed(), { used: 1, cap: 3 })).toBe(
      `Licensed to Ada Lovelace · updates included through ${through} · 1 of 3 devices activated.`,
    );
    expect(statusHintFor(licensed({ name: "" }), null)).toBe(
      `Licensed to owner@example.com · updates included through ${through}.`,
    );
    expect(statusInlineFor(licensed())).toBeNull();
  });

  test("no update expiry (i64::MAX sentinel) never prints Invalid Date", () => {
    expect(statusHintFor(licensed({ updateThroughMs: 9223372036854775807 }), null)).toBe(
      "Licensed to Ada Lovelace · updates included.",
    );
  });

  test("out of window: drops the update clause, warns inline", () => {
    const status = licensed({ inWindow: false });
    expect(statusHintFor(status, { used: 1, cap: 3 })).toBe("Licensed to Ada Lovelace · 1 of 3 devices activated.");
    expect(statusInlineFor(status)).toMatchObject({ tone: "warn", icon: "info" });
    expect(statusInlineFor(status)?.text).toContain(through);
  });

  test("pending / trial copy", () => {
    expect(statusHintFor(licensedPending(1), null)).toBe(
      "Licensed to Ada Lovelace. Finishing activation… (1 day to connect).",
    );
    expect(statusHintFor(trial(3), null)).toStartWith("3 days left · ends ");
    expect(statusHintFor(TRIAL_NOT_STARTED, null)).toBe("Free trial — starts when you first record (30 days).");
  });

  test("states that speak only through the inline / notice", () => {
    for (const status of [READ_ONLY, REVOKED, LICENSED_LAPSED, licensedOverCap(), null]) {
      expect(statusHintFor(status, { used: 3, cap: 3 })).toBeNull();
    }
    expect(statusInlineFor(READ_ONLY)).toMatchObject({ tone: "warn", icon: "lock" });
    expect(statusInlineFor(REVOKED)).toMatchObject({ tone: "danger", icon: "alert" });
    expect(statusInlineFor(LICENSED_LAPSED)).toMatchObject({ tone: "warn", icon: "cloud" });
    expect(statusInlineFor(licensedOverCap())).toBeNull();
  });
});

describe("safeExternalUrl (server-provided reset/buy links)", () => {
  test("https passes through", () => {
    expect(safeExternalUrl("https://mnema.day/#pricing")).toBe("https://mnema.day/#pricing");
  });

  test("non-https schemes are refused", () => {
    expect(safeExternalUrl("http://mnema.day/")).toBeNull();
    expect(safeExternalUrl("file:///etc/passwd")).toBeNull();
    expect(safeExternalUrl("mnema://license/activate?key=x")).toBeNull();
    expect(safeExternalUrl("javascript:alert(1)")).toBeNull();
  });

  test("garbage is refused, not thrown", () => {
    expect(safeExternalUrl("")).toBeNull();
    expect(safeExternalUrl("not a url")).toBeNull();
  });
});

describe("statusLineFor (onboarding license body)", () => {
  test("no status yet → null (generic explainer only)", () => {
    expect(statusLineFor(null)).toBeNull();
  });

  test("trial with day count, singular and plural", () => {
    expect(statusLineFor(trial(5))).toBe("You're on the free trial — 5 days left.");
    expect(statusLineFor(trial(1))).toBe("You're on the free trial — 1 day left.");
  });

  test("lapsed activation never promises 'never pauses'", () => {
    const line = statusLineFor(LICENSED_LAPSED);
    expect(line).toContain("activation hasn't finished");
    expect(line).not.toContain("never pauses");
  });

  test("owned + working activation thanks the owner", () => {
    for (const status of [licensed(), licensedPending(5), licensedOverCap()]) {
      expect(statusLineFor(status)).toContain("Recording never pauses.");
    }
  });

  test("readOnly and revoked are distinct, honest lines", () => {
    expect(statusLineFor(READ_ONLY)).toContain("trial has ended");
    expect(statusLineFor(REVOKED)).toContain("revoked");
    expect(statusLineFor(TRIAL_NOT_STARTED)).toContain("starts the moment you first record");
  });

  test("every wire variant has a line", () => {
    for (const status of ALL_VARIANTS) {
      expect(statusLineFor(status)).toBeTruthy();
    }
  });
});
