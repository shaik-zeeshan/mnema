// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { afterEach, describe, expect, test } from "bun:test";

// `cli-access.svelte.ts` is a runes module, but every `$state(...)` call sits
// inside `createCliAccessStore()`, so an identity shim is enough to exercise the
// store's load/error branches under plain `bun test` (no Svelte compile step).
globalThis.$state = (v) => v;

// `invoke` reads `window.__TAURI_INTERNALS__`, so the backend is stubbed by
// swapping that in — no module mock, so no process-wide export pinning.
function withBackend(handler) {
  globalThis.window = { __TAURI_INTERNALS__: { invoke: handler } };
}

afterEach(() => {
  delete globalThis.window;
});

const { createCliAccessStore } = await import("./cli-access.svelte");

describe("block / enable", () => {
  // Inverting this boolean makes the Block button un-block a tool and the Enable
  // button block it — a silent privacy inversion no other test can see, since
  // both directions otherwise look identical (same payload, same reload).
  test("each direction invokes its own command with the row's normalized label", async () => {
    const calls = [];
    withBackend(async (cmd, args) => {
      calls.push([cmd, args]);
      return cmd === "list_cli_access_grants" ? { grants: [] } : true;
    });
    const store = createCliAccessStore();
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
    withBackend(async () => ({ events: [] }));
    const store = createCliAccessStore();
    await store.loadBrokerHistory();
    expect(store.brokerHistory).toEqual([]);
    expect(store.brokerHistoryError).toBeNull();
  });

  test("a failed audit read is NOT reported as an empty log", async () => {
    withBackend(async () => {
      throw "failed to load CLI Access history: expected value at line 1 column 1";
    });
    const store = createCliAccessStore();
    await store.loadBrokerHistory();
    // The panel renders "Nothing yet." off an empty list, so an unreadable audit
    // file must leave a distinguishable error — otherwise a privacy surface
    // affirmatively tells the user nothing ran.
    expect(store.brokerHistory).toEqual([]);
    expect(store.brokerHistoryError).toBeTruthy();
  });
});
