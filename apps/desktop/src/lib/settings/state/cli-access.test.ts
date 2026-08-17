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

// ── Overlapping reloads ─────────────────────────────────────────────────────

// Hands every invoke back to the test as a promise it settles by hand, so the
// test — not the event loop — decides which in-flight read lands first. The
// panel fires each loader from four places (30 s poll, window-focus refetch, the
// Refresh button, and `setGrantBlocked`'s reload), so two reads overlapping is
// the normal case, not a contrived one.
function deferredInvokes() {
  const pending = [];
  const invokeFn = (cmd) => new Promise((resolve, reject) => pending.push({ cmd, resolve, reject }));
  return { invokeFn, pending };
}

const activeGrant = {
  id: "g1",
  label: "Claude Code",
  normalizedLabel: "claude-code",
  createdAtUnixMs: 1,
  lastUsedAtUnixMs: 2,
  scope: "all_retained_history",
  blocked: false,
  blockedAtUnixMs: null,
};

describe("overlapping reloads", () => {
  // Each loader clears its error slot when it STARTS, not when it succeeds. So an
  // older read that fails after a newer one has already painted the list leaves
  // the panel contradicting itself: a red "could not read the permission list"
  // sitting directly above that very list. On a privacy surface that reads as
  // "these permissions may be stale/wrong", and the only recovery the user has is
  // to hit Refresh until two reads happen not to overlap.
  test("a stale failed read does not report an error over a list that already loaded", async () => {
    const { invokeFn, pending } = deferredInvokes();
    const store = createCliAccessStore(invokeFn);

    const stale = store.loadBrokerGrants();
    const fresh = store.loadBrokerGrants();

    pending[1].resolve({ grants: [activeGrant] });
    await fresh;
    pending[0].reject("failed to read CLI Access grants");
    await stale;

    expect(store.brokerGrantError).toBeNull();
    expect(store.brokerGrants.map((grant) => grant.id)).toEqual(["g1"]);
  });

  // The reload `setGrantBlocked` fires races the 30 s poll that was already in
  // flight when the user clicked Block. If the poll's older, pre-block list lands
  // last it wins the slot, and the tool the user just blocked reappears as active
  // with a Block button — the click looks like it did nothing, and the user's
  // second click un-blocks it.
  test("a stale successful read does not overwrite a newer grant list", async () => {
    const { invokeFn, pending } = deferredInvokes();
    const store = createCliAccessStore(invokeFn);

    const stale = store.loadBrokerGrants();
    const fresh = store.loadBrokerGrants();

    pending[1].resolve({ grants: [{ ...activeGrant, blocked: true, blockedAtUnixMs: 3 }] });
    await fresh;
    pending[0].resolve({ grants: [activeGrant] });
    await stale;

    expect(store.brokerGrants.map((grant) => grant.blocked)).toEqual([true]);
  });

  // The spinner is the panel's only signal that the list on screen is not the
  // final answer. If an older response clears it, the newer read finishes into a
  // panel that already claimed to be settled — the rows visibly change under a
  // user who was told the read was done.
  test("a stale response does not stop the spinner while a newer read is still running", async () => {
    const { invokeFn, pending } = deferredInvokes();
    const store = createCliAccessStore(invokeFn);

    const stale = store.loadBrokerGrants();
    const fresh = store.loadBrokerGrants();

    pending[0].resolve({ grants: [] });
    await stale;
    expect(store.brokerGrantLoading).toBe(true);

    pending[1].resolve({ grants: [activeGrant] });
    await fresh;
    expect(store.brokerGrantLoading).toBe(false);
  });

  // Same self-contradiction as the grant list, on the evidence half of the panel:
  // an older failure landing last puts "could not read the activity log" above a
  // log that just loaded fine. Activity is what a user checks to see what a tool
  // actually read, so an error over a populated list makes them distrust the
  // rows rather than the read.
  test("a stale failed activity read does not report an error over a log that already loaded", async () => {
    const { invokeFn, pending } = deferredInvokes();
    const store = createCliAccessStore(invokeFn);
    const event = {
      toolIdentity: "Claude Code",
      commandType: "search",
      timestampUnixMs: 7,
      resultCount: 3,
      scopeClass: "time_scoped",
      outcome: null,
    };

    const stale = store.loadBrokerHistory();
    const fresh = store.loadBrokerHistory();

    pending[1].resolve({ events: [event] });
    await fresh;
    pending[0].reject("failed to load CLI Access history: expected value at line 1 column 1");
    await stale;

    expect(store.brokerHistoryError).toBeNull();
    expect(store.brokerHistory.map((row) => row.timestampUnixMs)).toEqual([7]);
  });

  // The mirror of the two grant cases: an older activity read must land nowhere
  // at all. Its list would show the log as it was BEFORE the command the user is
  // looking for, and its `finally` would stop the spinner on a read still in
  // flight — so "Nothing yet." can appear, settled, over an account that just ran
  // a tool.
  test("a stale activity response neither paints its rows nor stops the newer read", async () => {
    const { invokeFn, pending } = deferredInvokes();
    const store = createCliAccessStore(invokeFn);
    const row = (timestampUnixMs) => ({
      toolIdentity: "Claude Code",
      commandType: "search",
      timestampUnixMs,
      resultCount: 1,
      scopeClass: "time_scoped",
      outcome: null,
    });

    const stale = store.loadBrokerHistory();
    const fresh = store.loadBrokerHistory();

    pending[0].resolve({ events: [row(1)] });
    await stale;
    expect(store.brokerHistory).toEqual([]);
    expect(store.brokerHistoryLoading).toBe(true);

    pending[1].resolve({ events: [row(1), row(2)] });
    await fresh;
    expect(store.brokerHistory.map((event) => event.timestampUnixMs)).toEqual([2, 1]);
    expect(store.brokerHistoryLoading).toBe(false);
  });
});
