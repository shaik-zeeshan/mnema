#!/usr/bin/env bun
// Local mint CLI for re-mints (lost-key reissue) and comp (gifted) keys.
// Reuses src/mint.ts signing — see that file for the wire format. Seed is read
// from env ED25519_PRIVATE_KEY (base64 of the raw 32-byte seed); never persisted.
//
// Usage:
//   ED25519_PRIVATE_KEY=<b64-seed> bun scripts/mint-local.ts \
//     --order-id <polar_order_id> --email <e> --order-date <ISO-8601 | unix-ms>
//   ED25519_PRIVATE_KEY=<b64-seed> bun scripts/mint-local.ts \
//     --comp <slug> --email <e> [--update-days 90]
//
// Prints ONLY the key to stdout; a one-line summary goes to stderr. Delivery is manual.

import { mintKey, type LicensePayload } from "../src/mint";
import { base64ToBytes } from "../src/util";

const DAY_MS = 24 * 60 * 60 * 1000;

export function buildRemintPayload(args: {
  orderId: string;
  email: string;
  orderDateMs: number;
}): LicensePayload {
  return {
    email: args.email,
    license_id: "order:" + args.orderId,
    tier: "license",
    // Window dates from the ORIGINAL order date, NEVER from now — a re-mint must
    // not extend the buyer's Update Window (same id → still covered by revocation).
    issued_at: args.orderDateMs,
    update_through: args.orderDateMs + 365 * DAY_MS,
  };
}

export function buildCompPayload(args: {
  slug: string;
  email: string;
  updateDays?: number;
  now?: number;
}): LicensePayload {
  const now = args.now ?? Date.now();
  const days = args.updateDays ?? 90;
  return {
    email: args.email,
    license_id: "comp:" + args.slug,
    tier: "license",
    issued_at: now,
    update_through: now + days * DAY_MS,
  };
}

// Parse an ISO-8601 string or a unix-ms integer into unix ms.
export function parseOrderDate(raw: string): number {
  if (/^\d+$/.test(raw.trim())) return Number(raw.trim());
  const ms = Date.parse(raw);
  if (Number.isNaN(ms)) throw new Error(`--order-date is not ISO-8601 or unix-ms: ${raw}`);
  return ms;
}

function parseFlags(argv: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) throw new Error(`unexpected argument: ${a}`);
    const val = argv[i + 1];
    if (val === undefined || val.startsWith("--")) throw new Error(`missing value for ${a}`);
    out[a.slice(2)] = val;
    i++;
  }
  return out;
}

function die(msg: string): never {
  process.stderr.write(`error: ${msg}\n`);
  process.exit(1);
}

async function main() {
  const seedB64 = process.env.ED25519_PRIVATE_KEY;
  if (!seedB64) {
    die("ED25519_PRIVATE_KEY not set — export the base64 seed from the seller's password manager");
  }
  const seed = base64ToBytes(seedB64);

  let flags: Record<string, string>;
  try {
    flags = parseFlags(process.argv.slice(2));
  } catch (e) {
    die((e as Error).message);
  }

  const hasOrder = "order-id" in flags;
  const hasComp = "comp" in flags;
  if (hasOrder === hasComp) {
    die("choose exactly one mode: --order-id <id> (re-mint) OR --comp <slug> (comp)");
  }

  let payload: LicensePayload;
  if (hasOrder) {
    if (!flags.email) die("--email is required");
    if (!flags["order-date"]) die("--order-date is required (ISO-8601 or unix-ms, the ORIGINAL order date)");
    let orderDateMs: number;
    try {
      orderDateMs = parseOrderDate(flags["order-date"]);
    } catch (e) {
      die((e as Error).message);
    }
    payload = buildRemintPayload({ orderId: flags["order-id"], email: flags.email, orderDateMs });
  } else {
    if (!flags.email) die("--email is required");
    let updateDays: number | undefined;
    if (flags["update-days"] !== undefined) {
      updateDays = Number(flags["update-days"]);
      if (!Number.isFinite(updateDays) || updateDays <= 0) die("--update-days must be a positive number");
    }
    payload = buildCompPayload({ slug: flags.comp, email: flags.email, updateDays });
  }

  const key = await mintKey(payload, seed);
  process.stderr.write(
    `minted ${payload.license_id} for ${payload.email} — update_through ${new Date(payload.update_through).toISOString()}\n`,
  );
  process.stdout.write(key + "\n");
}

// Run only as a CLI (not when imported by the test).
if (import.meta.main) {
  main();
}
