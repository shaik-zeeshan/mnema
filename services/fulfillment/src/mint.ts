import * as ed from "@noble/ed25519";
import { bytesToBase64 } from "./util";

// License key wire format — MUST match the app verifier byte-for-byte.
// Verified to match crates/app-infra/src/license_verify.rs (same format: standard
// base64 halves joined by ".", signature over the raw payload_json UTF-8 bytes,
// ms timestamps).
//
//   key = base64(payload_json) + "." + base64(signature)
//
//   payload_json = compact JSON of, in this exact field order:
//     { email, license_id, tier, issued_at, update_through }
//   with issued_at / update_through as unix epoch MILLISECONDS (numbers).
//
//   signature   = Ed25519 over the raw payload_json UTF-8 bytes.
//   base64      = standard, with padding (NOT url-safe), for both halves.

export interface LicensePayload {
  email: string;
  license_id: string;
  tier: string;
  issued_at: number; // unix epoch ms
  update_through: number; // unix epoch ms
}

// `seed` is the raw 32-byte Ed25519 private seed.
export async function mintKey(payload: LicensePayload, seed: Uint8Array): Promise<string> {
  // Object-literal insertion order fixes the JSON field order; JSON.stringify is compact.
  const json = JSON.stringify({
    email: payload.email,
    license_id: payload.license_id,
    tier: payload.tier,
    issued_at: payload.issued_at,
    update_through: payload.update_through,
  });
  const bytes = new TextEncoder().encode(json);
  const signature = await ed.signAsync(bytes, seed);
  return `${bytesToBase64(bytes)}.${bytesToBase64(signature)}`;
}
