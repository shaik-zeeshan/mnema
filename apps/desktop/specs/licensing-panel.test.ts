// License panel policy (licensing-panel.ts): the money path (buy vs renew),
// badge, external-URL vetting, and the onboarding status line — plus the
// RENEWAL_CHECKOUT_URL `?`/`&` join in licensing.ts.

import { describe, expect, test } from "bun:test";
import {
  badgeFor,
  checkoutUrlFor,
  licensedOutOfWindow,
  safeExternalUrl,
  showBuyFor,
  statusLineFor,
} from "../src/lib/licensing-panel";
import {
  LICENSE_CHECKOUT_URL,
  RENEWAL_CHECKOUT_URL,
  renewalCheckoutUrl,
} from "../src/lib/licensing";
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

  test("renewalCheckoutUrl joins with ? on a bare URL and & when a query exists", () => {
    expect(renewalCheckoutUrl("https://x.io/checkout", "p1")).toBe(
      "https://x.io/checkout?product_id=p1",
    );
    expect(renewalCheckoutUrl("https://x.io/checkout?a=b", "p1")).toBe(
      "https://x.io/checkout?a=b&product_id=p1",
    );
  });

  test("default RENEWAL_CHECKOUT_URL carries the preselected product", () => {
    expect(RENEWAL_CHECKOUT_URL).toContain("product_id=");
    expect(RENEWAL_CHECKOUT_URL.startsWith(LICENSE_CHECKOUT_URL)).toBe(true);
  });
});

describe("badgeFor", () => {
  test("per-variant labels", () => {
    expect(badgeFor(licensed())).toEqual({ label: "Licensed", variant: "ok" });
    expect(badgeFor(licensedPending(5))).toEqual({ label: "Activating…", variant: "neutral" });
    expect(badgeFor(licensedOverCap())).toEqual({ label: "Device limit", variant: "warn" });
    expect(badgeFor(LICENSED_LAPSED)).toEqual({ label: "Not activated", variant: "warn" });
    expect(badgeFor(trial(3))).toEqual({ label: "Trial", variant: "neutral" });
    expect(badgeFor(TRIAL_NOT_STARTED)).toEqual({ label: "Trial ready", variant: "neutral" });
    expect(badgeFor(READ_ONLY)).toEqual({ label: "Read-only", variant: "warn" });
    expect(badgeFor(REVOKED)).toEqual({ label: "Revoked", variant: "warn" });
    expect(badgeFor(null)).toBeNull();
  });

  test("every wire variant produces a badge", () => {
    for (const status of ALL_VARIANTS) {
      expect(badgeFor(status)).not.toBeNull();
    }
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
