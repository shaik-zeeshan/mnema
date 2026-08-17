// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";

// A runes module, but every `$state(...)` sits inside `createCliAccessStore()`,
// which these tests never call — the helpers below are plain functions.
// Plain assignment, not `??=`: reading `globalThis.$state` throws
// `rune_outside_svelte` once svelte's client runtime has been loaded by an
// earlier file in the same bun process, so the read half is the trap.
globalThis.$state = (v) => v;

const {
  formatGrantScope,
  formatOutcome,
  formatCommand,
  formatActivityDetail,
  grantStatus,
  grantStatusLabel,
} = await import("./cli-access.svelte");

const grant = (over) => ({
  id: "g1",
  label: "Claude Code",
  normalizedLabel: "claude code",
  createdAtUnixMs: 0,
  lastUsedAtUnixMs: 0,
  scope: "all_retained_history",
  blocked: false,
  blockedAtUnixMs: null,
  ...over,
});

describe("scope rendering", () => {
  // The exact wire shape `app_infra::brokered_access::BrokerGrantScope` emits:
  // an externally-tagged enum under `rename_all = "snake_case"`. The Rust half of
  // this contract is pinned in `brokered_access/tests.rs`
  // (`broker_grant_scope_wire_shape_is_what_settings_decodes`); if either side
  // moves alone, every permission row silently renders "Limited scope".
  test("both real wire shapes render their scope, not the fallback", () => {
    expect(formatGrantScope("all_retained_history")).toBe("All retained history");
    expect(formatGrantScope({ recent_days: { days: 1 } })).toBe("Last day");
    expect(formatGrantScope({ recent_days: { days: 7 } })).toBe("Last 7 days");
  });

  test("a shape this build does not know degrades instead of throwing", () => {
    expect(formatGrantScope({ forever: {} })).toBe("Limited scope");
    expect(formatGrantScope(null)).toBe("Limited scope");
  });
});

describe("permission row status", () => {
  // The relative-time half is `Intl.RelativeTimeFormat` on the SYSTEM locale, so
  // the wording is asserted by shape, not by an English string this suite would
  // fail on under any other locale.
  test("blocked dates the block; active dates its last use", () => {
    const now = Date.UTC(2026, 0, 10);
    const dayAgo = now - 24 * 60 * 60 * 1000;

    expect(grantStatus(grant({ blocked: true }))).toBe("blocked");
    expect(grantStatus(grant())).toBe("active");

    // Both rows label their own timestamp. The name line carries a BLOCKED
    // badge, but a bare relative time beside an active row's "Used 3 hours ago"
    // would leave the reader unable to tell last-use from block-time.
    const blocked = grantStatusLabel(grant({ blocked: true, blockedAtUnixMs: dayAgo }), now);
    expect(blocked.startsWith("Blocked ")).toBe(true);
    expect(blocked).not.toBe("Blocked");

    // The active row must date the LAST USE, not the block slot — reading the
    // wrong field is how every live tool would read "Used 56 years ago".
    const used = grantStatusLabel(grant({ lastUsedAtUnixMs: dayAgo }), now);
    expect(used).toBe(`Used ${blocked.slice("Blocked ".length)}`);
  });

  // `blockedAtUnixMs` is `Option<u64>` on the wire, so `null` is reachable for
  // any row blocked by a build that did not stamp it. `56 years ago` is what a
  // naive read of a null gives; the row drops the time cell instead.
  // The BLOCKED badge on the name line already states the state, so an unstamped
  // block adds nothing here — and must never render a date from the epoch, which
  // is what a naive read of the null gives ("Blocked 56 years ago").
  test("a block with no timestamp adds nothing, never a date from the epoch", () => {
    expect(grantStatusLabel(grant({ blocked: true, blockedAtUnixMs: null }), Date.UTC(2026, 0, 10))).toBe(
      "",
    );
  });
});

describe("activity rows", () => {
  test("a refusal is named; a success shows its result count", () => {
    expect(formatOutcome("denied")).toBe("Denied");
    expect(formatOutcome("scope_rejected")).toBe("Out of scope");
    // A success carries no word — the row shows its count instead.
    expect(formatOutcome("success")).toBe("");
    expect(formatOutcome(null)).toBe("");
    // An outcome a newer backend writes must degrade to a word a user can read,
    // not leak the raw snake_case wire value the way the scope fallback would not.
    expect(formatOutcome("rate_limited")).toBe("Refused");
  });

  test("the refusal wins over the count, and 1 result is singular", () => {
    const event = (over) => ({
      toolIdentity: "Claude Code",
      commandType: "search",
      timestampUnixMs: 0,
      resultCount: 0,
      scopeClass: "time_scoped",
      outcome: null,
      ...over,
    });

    // A denied request reports 0 results; showing "0 results" instead of
    // "Denied" is the row claiming the tool looked and found nothing.
    expect(formatActivityDetail(event({ outcome: "denied", resultCount: 0 }))).toBe("Denied");
    expect(formatActivityDetail(event({ resultCount: 1 }))).toBe("1 result");
    expect(formatActivityDetail(event({ resultCount: 0 }))).toBe("0 results");
    expect(formatActivityDetail(event({ outcome: "success", resultCount: 4 }))).toBe("4 results");
  });

  test("the command reads as the subcommand the tool actually ran", () => {
    expect(formatCommand("show_text")).toBe("show-text");
    expect(formatCommand("open_in_mnema")).toBe("open-in-mnema");
    expect(formatCommand("search")).toBe("search");
  });
});
