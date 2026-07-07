// Standard base64 (with padding) <-> bytes. NOT url-safe — the app verifier
// (crates/app-infra/src/license_verify.rs) decodes standard base64.
// btoa/atob exist in Cloudflare Workers and Bun.

export function bytesToBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

export function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}
