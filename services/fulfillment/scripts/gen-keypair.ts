#!/usr/bin/env bun
// Generate an Ed25519 licensing keypair (per-env keypair split, ADR 0045).
// Prints the seed (worker/mint secret), the public key (dev-build override or
// prod const), and the Rust byte-array literal for pasting into
// crates/app-infra/src/license_verify.rs when rotating the PRODUCTION key.
//
// Store the seed in the seller's password manager, NEVER in the repo. Use one
// keypair per environment: the prod seed is the worker's ED25519_PRIVATE_KEY
// under `--env production`; the dev seed under `--env dev`, and the dev PUBLIC
// key becomes MNEMA_LICENSE_PUBLIC_KEY when building the dev desktop app.
//
// Usage:  bun scripts/gen-keypair.ts

import * as ed from "@noble/ed25519";
import { bytesToBase64 } from "../src/util";

const seed = crypto.getRandomValues(new Uint8Array(32));
const pub = await ed.getPublicKeyAsync(seed);

const rustArray =
  "    " +
  Array.from(pub, (b, i) => `0x${b.toString(16).padStart(2, "0")},${(i + 1) % 16 === 0 ? "\n    " : " "}`)
    .join("")
    .trimEnd();

process.stdout.write(
  [
    "Ed25519 licensing keypair — store the seed in a password manager, never commit it.",
    "",
    `ED25519_PRIVATE_KEY (base64 seed):   ${bytesToBase64(seed)}`,
    `  → worker secret:  wrangler secret put ED25519_PRIVATE_KEY --env <dev|production>`,
    `  → local mint:     ED25519_PRIVATE_KEY=<above> bun scripts/mint-local.ts ...`,
    "",
    `MNEMA_LICENSE_PUBLIC_KEY (base64):   ${bytesToBase64(pub)}`,
    `  → dev desktop build: export MNEMA_LICENSE_PUBLIC_KEY=<above>  (see scripts/dev-app.sh)`,
    "",
    "Rust literal — paste into PRODUCTION_LICENSE_PUBLIC_KEY only when rotating PROD:",
    `const PRODUCTION_LICENSE_PUBLIC_KEY: [u8; 32] = [`,
    rustArray,
    `];`,
    "",
  ].join("\n"),
);
