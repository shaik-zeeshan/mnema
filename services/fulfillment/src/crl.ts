import * as ed from "@noble/ed25519";
import { bytesToBase64, base64ToBytes } from "./util";

// Revocation List (CRL) wire format — MUST match the app verifier byte-for-byte
// (crates/app-infra CRL core, ADR 0052).
//
//   wire = base64(payload_json) + "." + base64(signature)
//
//   payload_json = compact JSON, exact field order:
//     { schema: 1, issued_at: <unix ms int>, revoked_license_ids: [<string>...] }
//
//   signature = Ed25519 over the UTF-8 bytes of "mnema-crl-v1:" + payload_json.
//     The domain-separation prefix means a CRL can never replay as a license key
//     (which signs the raw payload with no prefix) or vice versa.
//   base64 = standard, with padding, for both halves.

const CRL_CONTEXT = "mnema-crl-v1:";

// Exact compact JSON with fixed field order (object-literal insertion order).
export function serializeCrlPayload(revokedIds: string[], issuedAt: number): string {
  return JSON.stringify({
    schema: 1,
    issued_at: issuedAt,
    revoked_license_ids: revokedIds,
  });
}

// `seed` is the raw 32-byte Ed25519 private seed (same as license keys).
// issued_at is monotonic: max(now, prevIssuedAt + 1) survives a clock stuck in
// the past across redeploys. Ids are sorted for stable output.
export async function buildAndSignCrl(
  revokedIds: string[],
  prevIssuedAt: number,
  now: number,
  seed: Uint8Array,
): Promise<string> {
  const issuedAt = Math.max(now, prevIssuedAt + 1);
  const sorted = [...revokedIds].sort();
  const json = serializeCrlPayload(sorted, issuedAt);
  const signedBytes = new TextEncoder().encode(CRL_CONTEXT + json);
  const signature = await ed.signAsync(signedBytes, seed);
  return `${bytesToBase64(new TextEncoder().encode(json))}.${bytesToBase64(signature)}`;
}

function parsePayload(wire: string): { schema: number; issued_at: number; revoked_license_ids: string[] } {
  const payloadB64 = wire.split(".")[0];
  const json = new TextDecoder().decode(base64ToBytes(payloadB64));
  return JSON.parse(json);
}

// Read prev state from a stored wire string WITHOUT verifying — the worker owns
// the key, so this is just to recover the previous issued_at / id set from KV.
export function crlIssuedAt(wire: string): number {
  return parsePayload(wire).issued_at;
}

export function crlRevokedIds(wire: string): string[] {
  return parsePayload(wire).revoked_license_ids;
}
