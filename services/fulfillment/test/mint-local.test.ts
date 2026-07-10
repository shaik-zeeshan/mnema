import { test, expect } from "bun:test";
import * as ed from "@noble/ed25519";
import { mintKey } from "../src/mint";
import { base64ToBytes } from "../src/util";
import { buildRemintPayload, buildCompPayload, parseOrderDate } from "../scripts/mint-local";

const DAY_MS = 24 * 60 * 60 * 1000;

// --- re-mint payload -------------------------------------------------------

test("re-mint: id order:<id>, window is 365d dated from the ORIGINAL order date (not now)", () => {
  const orderDateMs = 1_700_000_000_000;
  const p = buildRemintPayload({ orderId: "ord_abc", email: "buyer@x.io", orderDateMs });

  expect(p.license_id).toBe("order:ord_abc");
  expect(p.tier).toBe("license");
  expect(p.issued_at).toBe(orderDateMs); // window dates from order date, not now
  expect(p.update_through - p.issued_at).toBe(365 * DAY_MS);
});

test("parseOrderDate accepts ISO-8601 and unix-ms alike", () => {
  expect(parseOrderDate("1700000000000")).toBe(1_700_000_000_000);
  expect(parseOrderDate("2023-11-14T22:13:20.000Z")).toBe(Date.parse("2023-11-14T22:13:20.000Z"));
});

// --- comp payload ----------------------------------------------------------

test("comp: id comp:<slug>, default window is 90d", () => {
  const now = 1_700_000_000_000;
  const p = buildCompPayload({ slug: "press-kit", email: "friend@x.io", now });

  expect(p.license_id).toBe("comp:press-kit");
  expect(p.tier).toBe("license");
  expect(p.issued_at).toBe(now);
  expect(p.update_through - p.issued_at).toBe(90 * DAY_MS);
});

test("comp: --update-days overrides the default", () => {
  const now = 1_700_000_000_000;
  const p = buildCompPayload({ slug: "vip", email: "vip@x.io", updateDays: 365, now });
  expect(p.update_through - p.issued_at).toBe(365 * DAY_MS);
});

// --- minted keys verify against the derived public key ---------------------
// One consolidated test: mintKey's signature scheme itself is pinned by the
// cross-language fixture in fulfillment.test.ts; here we only prove that keys
// minted from BOTH locally-built payload shapes verify and carry the derived id.

test("re-mint and comp keys verify and carry their derived ids", async () => {
  const seed = crypto.getRandomValues(new Uint8Array(32));
  const pub = await ed.getPublicKeyAsync(seed);

  const payloads = [
    { payload: buildRemintPayload({ orderId: "ord_sig", email: "b@x.io", orderDateMs: 1_700_000_000_000 }), id: "order:ord_sig" },
    { payload: buildCompPayload({ slug: "gift-01", email: "g@x.io" }), id: "comp:gift-01" },
  ];
  for (const { payload, id } of payloads) {
    const key = await mintKey(payload, seed);
    const [payloadB64, sigB64] = key.split(".");
    const payloadBytes = base64ToBytes(payloadB64);
    expect(await ed.verifyAsync(base64ToBytes(sigB64), payloadBytes, pub)).toBe(true);
    expect(JSON.parse(new TextDecoder().decode(payloadBytes)).license_id).toBe(id);
  }
});
