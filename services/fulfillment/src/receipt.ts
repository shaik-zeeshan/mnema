import * as ed from "@noble/ed25519";
import { bytesToBase64 } from "./util";

// Activation Receipt wire format — MUST match the app verifier byte-for-byte
// (mirrors crl.ts, different domain + payload). ADR 0053.
//
//   wire = base64(payload_json) + "." + base64(signature)
//
//   payload_json = compact JSON, exact field order:
//     { schema: 1, license_id: <string>, machine_hash: <hex string>,
//       activated_at: <unix ms int> }
//
//   signature = Ed25519 over the UTF-8 bytes of "mnema-receipt-v1:" + payload_json.
//     The domain-separation prefix keeps a receipt from replaying as a license key
//     (raw payload, no prefix) or a CRL ("mnema-crl-v1:"), and vice versa.
//   base64 = standard, with padding, for both halves.
//   machine_hash is opaque to the worker (computed app-side) — never recomputed.
//   No expiry field: a receipt is offline-valid forever.

const RECEIPT_CONTEXT = "mnema-receipt-v1:";

// Exact compact JSON with fixed field order (object-literal insertion order).
export function serializeReceiptPayload(
  licenseId: string,
  machineHash: string,
  activatedAt: number,
): string {
  return JSON.stringify({
    schema: 1,
    license_id: licenseId,
    machine_hash: machineHash,
    activated_at: activatedAt,
  });
}

// `seed` is the raw 32-byte Ed25519 private seed (same as license keys).
export async function signReceipt(
  licenseId: string,
  machineHash: string,
  activatedAt: number,
  seed: Uint8Array,
): Promise<string> {
  const json = serializeReceiptPayload(licenseId, machineHash, activatedAt);
  const signedBytes = new TextEncoder().encode(RECEIPT_CONTEXT + json);
  const signature = await ed.signAsync(signedBytes, seed);
  return `${bytesToBase64(new TextEncoder().encode(json))}.${bytesToBase64(signature)}`;
}
