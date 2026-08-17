// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";

// `cli-access.svelte.ts` is a runes module, but every `$state(...)` call sits
// inside `createCliAccessStore()`, so an identity shim is enough to exercise the
// store's load/error branches under plain `bun test` (no Svelte compile step).
// Plain assignment, never `??=`: once svelte's client runtime is loaded (any
// earlier file in a full run), READING `globalThis.$state` throws
// `rune_outside_svelte`, so the compound assignment blows up before it can
// short-circuit.
globalThis.$state = (v) => v;

const { createCliAccessStore } = await import("./cli-access.svelte");

// The backend is handed to the store as its `invokeFn` seam. Stubbing
// `window.__TAURI_INTERNALS__` instead would pass ONLY when this file runs alone:
// bun's `mock.module` is process-wide and several `specs/*.test.ts` register an
// `@tauri-apps/api/core` mock, so in a full run the module import — not the
// window — is what the store would have called.

describe("block / enable", () => {
  // Inverting this boolean makes the Block button un-block a tool and the Enable
  // button block it — a silent privacy inversion no other test can see, since
  // both directions otherwise look identical (same payload, same reload).
  test("each direction invokes its own command with the row's normalized label", async () => {
    const calls = [];
    const store = createCliAccessStore(async (cmd, args) => {
      calls.push([cmd, args]);
      return cmd === "list_cli_access_grants" ? { grants: [] } : true;
    });
    // `label` is what the user sees; `normalizedLabel` is the identity the
    // backend keys the permission row on — passing the display label would
    // block/enable nothing at all.
    const grant = { id: "g1", label: "Claude Code", normalizedLabel: "claude-code" };

    await store.setGrantBlocked(grant, true);
    await store.setGrantBlocked(grant, false);

    const writes = calls.filter(([cmd]) => cmd !== "list_cli_access_grants");
    expect(writes).toEqual([
      ["block_cli_access_client", { clientName: "claude-code" }],
      ["unblock_cli_access_client", { clientName: "claude-code" }],
    ]);
    expect(store.brokerGrantError).toBeNull();
  });
});

describe("activity log load", () => {
  test("an empty audit file reports an empty log and no error", async () => {
    const store = createCliAccessStore(async () => ({ events: [] }));
    await store.loadBrokerHistory();
    expect(store.brokerHistory).toEqual([]);
    expect(store.brokerHistoryError).toBeNull();
  });

  // The audit file is append-ordered (oldest first) and capped at 500; the panel
  // shows 20 under the heading "Recent activity". Without the reverse it shows the
  // OLDEST 20 — a privacy surface headed "recent" that omits everything that just
  // happened, which no other assertion here can see (the list is the right length
  // either way).
  test("the panel shows the newest events, not the oldest", async () => {
    const events = Array.from({ length: 25 }, (_, index) => ({
      toolIdentity: "Claude Code",
      commandType: "search",
      timestampUnixMs: index,
      resultCount: 0,
      scopeClass: "time_scoped",
      outcome: null,
    }));
    const store = createCliAccessStore(async () => ({ events }));
    await store.loadBrokerHistory();

    expect(store.brokerHistory.length).toBe(20);
    expect(store.brokerHistory[0].timestampUnixMs).toBe(24);
    expect(store.brokerHistory[19].timestampUnixMs).toBe(5);
    // The invoke result must not be mutated in place.
    expect(events[0].timestampUnixMs).toBe(0);
  });

  test("a failed audit read is NOT reported as an empty log", async () => {
    const store = createCliAccessStore(async () => {
      throw "failed to load CLI Access history: expected value at line 1 column 1";
    });
    await store.loadBrokerHistory();
    // The panel renders "Nothing yet." off an empty list, so an unreadable audit
    // file must leave a distinguishable error — otherwise a privacy surface
    // affirmatively tells the user nothing ran.
    expect(store.brokerHistory).toEqual([]);
    expect(store.brokerHistoryError).toBeTruthy();
  });
});
