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
  formatScopeChip,
  grantLapse,
  lastCommandByTool,
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

describe("idle lapse meter", () => {
  const DAY = 24 * 60 * 60 * 1000;
  const now = Date.UTC(2026, 9, 10);

  // last use + 30 days − now, rounded UP: a tool used 26 days ago has 4 days
  // left, and one used a minute ago still reads the full 30.
  test("counts down from the last use, not from creation", () => {
    const used26 = grantLapse(grant({ createdAtUnixMs: now - 90 * DAY, lastUsedAtUnixMs: now - 26 * DAY }), now);
    expect(used26.label).toBe("Lapses in 4 days");
    expect(used26.soon).toBe(true);
    expect(used26.fraction).toBeCloseTo(4 / 30);
    const fresh = grantLapse(grant({ lastUsedAtUnixMs: now - 60_000 }), now);
    expect(fresh.label).toBe("Lapses in 30 days");
    expect(fresh.soon).toBe(false);
  });

  // The backend prunes a row at 30 days, so a poll can briefly hold one at or
  // past the line: it reads "today" and an empty bar, never a negative count.
  test("the last day and an overdue row read 'today', never negative", () => {
    expect(grantLapse(grant({ lastUsedAtUnixMs: now - 29.5 * DAY }), now).label).toBe("Lapses today");
    const over = grantLapse(grant({ lastUsedAtUnixMs: now - 31 * DAY }), now);
    expect(over.label).toBe("Lapses today");
    expect(over.fraction).toBe(0);
  });

  test("scope chips are lowercase time windows", () => {
    expect(formatScopeChip("all_retained_history")).toBe("all retained");
    expect(formatScopeChip({ recent_days: { days: 7 } })).toBe("last 7 days");
    expect(formatScopeChip({ recent_days: { days: 1 } })).toBe("last day");
  });
});

describe("last ran", () => {
  const ev = (who, cmd, at) => ({ toolIdentity: who, normalizedToolIdentity: who, commandType: cmd, timestampUnixMs: at, resultCount: 0, scopeClass: "", outcome: null });

  // Keyed by the normalized identity the grant row carries, newest event wins
  // whatever the file order, and a refused command still counts as what it ran.
  test("newest command per tool, by normalized identity", () => {
    const map = lastCommandByTool([ev("codex", "search", 5), ev("claude code", "show_text", 9), ev("codex", "timeline", 7), ev("codex", "speakers", 6)]);
    expect(map.get("codex")).toBe("timeline");
    expect(map.get("claude code")).toBe("show-text");
    expect(map.size).toBe(2);
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
