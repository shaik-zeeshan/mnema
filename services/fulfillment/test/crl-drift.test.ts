import { test, expect } from "bun:test";
import defaultExport, { type Env } from "../src/index";
import { buildAndSignCrl, crlRevokedIds } from "../src/crl";
import { bytesToBase64 } from "../src/util";

const TEST_SEED = new Uint8Array(32).fill(9);

// GET /revocations.json rebuilds the CRL whenever the stored `crl` id set differs
// from what `list({prefix:"revoked:"})` returns. KV `list` is EVENTUALLY
// consistent and can lag across edges: a just-written revoked:* key may be
// invisible on the edge serving this GET, so `list` returns a SUBSET of the ids
// already baked into the durable, propagated CRL.
//
// When that happens the drift-detector fires (sets differ) and rebuilds the CRL
// from the stale subset — DROPPING a real revocation and re-signing with a higher
// issued_at. The app fetches this "newer" CRL and treats a refunded license as
// valid again. Revocations must be append-only: a rebuild can never shrink below
// the ids already present in the current signed CRL.
test("GET /revocations.json never drops a revocation the stored CRL already has", async () => {
  const store = new Map<string, string>();
  // Durable, propagated CRL already covers BOTH refunds.
  const fullCrl = await buildAndSignCrl(["order:A", "order:B"], 0, 1_700_000_000_000, TEST_SEED);
  store.set("crl", fullCrl);
  // Both revocations are durably written...
  store.set("revoked:order:A", "1");
  store.set("revoked:order:B", "1");

  // ...but THIS edge's `list` lags and only sees order:A (order:B not yet visible).
  const env = {
    ED25519_PRIVATE_KEY: bytesToBase64(TEST_SEED),
    IDEMPOTENCY: {
      get: async (k: string) => store.get(k) ?? null,
      put: async (k: string, v: string) => {
        store.set(k, v);
      },
      list: async ({ prefix }: { prefix: string }) => {
        const names = [...store.keys()].filter(
          (k) => k.startsWith(prefix) && k !== "revoked:order:B", // stale: B invisible
        );
        return { keys: names.map((name) => ({ name })), list_complete: true, cursor: undefined };
      },
    },
  } as unknown as Env;

  const res = await defaultExport.fetch(
    new Request("https://f.example/revocations.json", { method: "GET" }),
    env,
  );
  const served = crlRevokedIds((await res.text()).trim());
  // order:B was durably revoked and already in the signed CRL — it must survive.
  expect(new Set(served)).toEqual(new Set(["order:A", "order:B"]));
});
