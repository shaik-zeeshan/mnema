import { expect, test } from "bun:test";
import * as ed from "@noble/ed25519";
import { verifyCrlWire } from "../scripts/bake-crl";
import { buildAndSignCrl, serializeCrlPayload } from "../src/crl";
import { bytesToBase64 } from "../src/util";

// Test seed (NOT the production key) — the positive case verifies under the
// pubkey derived from this seed, mirroring what the worker signs with.
const TEST_SEED = new Uint8Array(32).fill(7);

test("verifyCrlWire accepts a well-formed, correctly-signed CRL", async () => {
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  const wire = await buildAndSignCrl(["order:abc"], 0, 1_700_000_000_000, TEST_SEED);
  expect(await verifyCrlWire(wire, pub)).toBe(true);
});

test("verifyCrlWire rejects a tampered payload", async () => {
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  const wire = await buildAndSignCrl(["order:abc"], 0, 1_700_000_000_000, TEST_SEED);
  const [, sigB64] = wire.split(".");
  // Swap in a different payload while keeping the original signature.
  const tamperedPayload = serializeCrlPayload(["order:evil"], 1_700_000_000_000);
  const tampered = `${bytesToBase64(new TextEncoder().encode(tamperedPayload))}.${sigB64}`;
  expect(await verifyCrlWire(tampered, pub)).toBe(false);
});

test("verifyCrlWire rejects a signature made WITHOUT the domain prefix (wrong context)", async () => {
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  const payloadJson = serializeCrlPayload(["order:abc"], 1_700_000_000_000);
  // Sign the raw payload (license context) instead of "mnema-crl-v1:" + payload.
  const sig = await ed.signAsync(new TextEncoder().encode(payloadJson), TEST_SEED);
  const wire = `${bytesToBase64(new TextEncoder().encode(payloadJson))}.${bytesToBase64(sig)}`;
  expect(await verifyCrlWire(wire, pub)).toBe(false);
});

test("verifyCrlWire rejects garbage / unparseable wire", async () => {
  const pub = await ed.getPublicKeyAsync(TEST_SEED);
  expect(await verifyCrlWire("not-a-wire", pub)).toBe(false);
  expect(await verifyCrlWire("", pub)).toBe(false);
});
