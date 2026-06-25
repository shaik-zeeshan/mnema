// The shared, debounced Settings autosave engine.
//
// ── Injected-closure contract (READ THIS BEFORE CHANGING THE ENGINE) ─────────
// The engine never reads draft `$state` or builds a save request itself. Every
// per-domain unit hands the engine a small bundle of CLOSURES:
//
//   • snapshot()      → the domain's current draft serialized to a stable string
//   • baseline()      → the last successfully-persisted snapshot (null = unloaded)
//   • blocked()       → true when domain-specific validation forbids saving
//   • saving()        → true while a save for this domain is already in flight
//   • save()          → run the actual persist (the only place `invoke` happens)
//
// plus one engine-wide predicate:
//
//   • privacyCommandInFlight() → true while the app-privacy controller holds a
//                                 command (suppresses all autosaves meanwhile)
//
// Because the engine only ever calls these closures, WHERE the draft state lives
// is invisible to it. Today the drafts are `+page.svelte`-local; when panels are
// extracted and drafts move into domain state modules, ONLY the registered
// closures change — the engine, its debounce logic, and its scheduling are
// untouched. Do not reach into draft state, build requests, or call `invoke`
// from this file; keep all of that behind the injected closures.
//
// The pure scheduling decision (dirty? blocked? in-flight? saving?) lives in
// `autosave-core.ts#shouldSaveDomain` so it can be unit-tested without a runtime.

import { shouldSaveDomain } from "./autosave-core";

// One autosave unit: a stable key plus the injected closures above. The key is
// the domain id for recording domains, or a synthetic id ("keyboard", "mic")
// for the other two autosaved surfaces — it only has to be unique per engine.
export interface AutosaveUnit {
  key: string;
  debounceMs: number;
  snapshot: () => string;
  baseline: () => string | null;
  blocked: () => boolean;
  saving: () => boolean;
  save: () => void | Promise<void>;
}

export interface AutosaveEngineHost {
  // Engine-wide suppression: while a privacy command is mid-flight no unit may
  // autosave (the privacy controller mutates the same recording settings).
  privacyCommandInFlight: () => boolean;
}

// Decide whether a unit may save right now (pure gate, runtime-free inputs).
function unitReady(unit: AutosaveUnit, host: AutosaveEngineHost): boolean {
  return shouldSaveDomain({
    current: unit.snapshot(),
    baseline: unit.baseline(),
    blocked: unit.blocked(),
    privacyCommandInFlight: host.privacyCommandInFlight(),
    saving: unit.saving(),
  });
}

// The engine. Construct once, register the units, then drive it from a single
// reactive `$effect` that READS the relevant snapshots (so Svelte re-runs the
// tick on any draft edit) and calls `engine.tick()`.
//
// `tick()` is intentionally framework-free: it does not itself subscribe to
// anything. The caller's `$effect` owns the reactive subscription by reading the
// snapshots before calling `tick()`. This keeps the engine testable with a fake
// clock and plain (non-reactive) units.
export function createAutosaveEngine(host: AutosaveEngineHost) {
  const units = new Map<string, AutosaveUnit>();
  const timers = new Map<string, ReturnType<typeof setTimeout>>();

  function register(unit: AutosaveUnit): void {
    units.set(unit.key, unit);
  }

  function clearTimer(key: string): void {
    const timer = timers.get(key);
    if (timer !== undefined) {
      clearTimeout(timer);
      timers.delete(key);
    }
  }

  // Re-evaluate every registered unit. For each: if it is not ready to save,
  // cancel any pending timer; otherwise (re)arm a debounce. When the debounce
  // fires, re-check readiness once more (state may have changed during the
  // wait) and only then run the unit's `save()`.
  function tick(): void {
    for (const unit of units.values()) {
      if (!unitReady(unit, host)) {
        clearTimer(unit.key);
        continue;
      }
      // Re-arm: a fresh edit during the debounce window restarts the timer.
      clearTimer(unit.key);
      const timer = setTimeout(() => {
        timers.delete(unit.key);
        // Re-check at fire time — the gate may have closed while we waited.
        if (!unitReady(unit, host)) return;
        void unit.save();
      }, unit.debounceMs);
      timers.set(unit.key, timer);
    }
  }

  // Cancel all pending debounces (used by the privacy controller before it
  // issues its own command, and on teardown).
  function cancelAll(): void {
    for (const key of [...timers.keys()]) clearTimer(key);
  }

  // Test/diagnostic helper: is a debounce currently armed for this unit?
  function hasPendingTimer(key: string): boolean {
    return timers.has(key);
  }

  return { register, tick, cancelAll, hasPendingTimer };
}

export type AutosaveEngine = ReturnType<typeof createAutosaveEngine>;
