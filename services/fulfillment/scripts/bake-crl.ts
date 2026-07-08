#!/usr/bin/env bun
// PLAN slice 6 — bake the live signed CRL over the committed placeholder before a
// release build. Fetches the seller-owned /revocations.json, verifies its Ed25519
// signature against the PRODUCTION public key under the domain-separated context,
// and only then overwrites crates/app-infra/revocations.json. Any problem (non-2xx,
// unparseable wire, bad signature) exits non-zero with a clear stderr message so the
// release fails loudly. The committed placeholder keeps offline/dev builds working.
//
// Usage:  [CRL_ENDPOINT=https://...] bun scripts/bake-crl.ts

import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as ed from "@noble/ed25519";
import { base64ToBytes } from "../src/util";
import { crlIssuedAt, crlRevokedIds } from "../src/crl";

const CRL_CONTEXT = "mnema-crl-v1:";
// TEMPORARY workers.dev endpoint — replace with the seller-owned custom domain
// (e.g. crl.mnema.app) before shipping. Build-time only, overridable via CRL_ENDPOINT.
const DEFAULT_ENDPOINT = "https://mnema-fulfillment.shaikzeeshan999.workers.dev/revocations.json";

// Production Ed25519 public key — must equal LICENSE_PUBLIC_KEY in
// crates/app-infra/src/license_verify.rs (kept in sync by hand).
export const PRODUCTION_PUBLIC_KEY = new Uint8Array([
  0x60, 0x7b, 0x11, 0x3a, 0x84, 0x46, 0x64, 0xe5, 0xfc, 0xae, 0x01, 0x84, 0xf0, 0xc9, 0x7e, 0xe4,
  0x6c, 0xde, 0x69, 0xb9, 0x40, 0xdd, 0xc6, 0xde, 0xa8, 0x46, 0xb8, 0x2f, 0x88, 0x4c, 0xe2, 0x12,
]);

// Pure, testable: does `wire` carry a valid domain-separated signature for `pubkey`?
// Never throws — a malformed wire is just "not verified".
export async function verifyCrlWire(wire: string, pubkey: Uint8Array): Promise<boolean> {
  try {
    const [payloadB64, sigB64] = wire.trim().split(".");
    if (!payloadB64 || !sigB64) return false;
    const payloadJson = new TextDecoder().decode(base64ToBytes(payloadB64));
    const signed = new TextEncoder().encode(CRL_CONTEXT + payloadJson);
    return await ed.verifyAsync(base64ToBytes(sigB64), signed, pubkey);
  } catch {
    return false;
  }
}

function die(msg: string): never {
  process.stderr.write(`bake-crl: ${msg}\n`);
  process.exit(1);
}

async function main() {
  const endpoint = process.env.CRL_ENDPOINT ?? DEFAULT_ENDPOINT;

  let res: Response;
  try {
    res = await fetch(endpoint);
  } catch (e) {
    die(`fetch failed for ${endpoint}: ${(e as Error).message}`);
  }
  if (!res.ok) die(`endpoint ${endpoint} returned HTTP ${res.status} ${res.statusText}`);

  const wire = (await res.text()).trim();
  if (!wire) die(`empty response body from ${endpoint}`);

  if (!(await verifyCrlWire(wire, PRODUCTION_PUBLIC_KEY))) {
    die(
      `signature does NOT verify against the production public key under "${CRL_CONTEXT}" — ` +
        `refusing to bake (endpoint ${endpoint})`,
    );
  }

  // Signature is good, so the payload is safe to read for the summary line.
  const issuedAt = crlIssuedAt(wire);
  const revokedCount = crlRevokedIds(wire).length;

  // Resolve relative to the repo root (this file is at services/fulfillment/scripts/).
  const repoRoot = resolve(fileURLToPath(new URL(".", import.meta.url)), "../../..");
  const outPath = resolve(repoRoot, "crates/app-infra/revocations.json");
  await Bun.write(outPath, wire);

  process.stdout.write(
    `bake-crl: baked CRL from ${endpoint} — issued_at=${issuedAt} revoked=${revokedCount} -> ${outPath}\n`,
  );
}

if (import.meta.main) {
  main();
}
