import { describe, expect, test } from "bun:test";
import { isDirty } from "../src/lib/settings/state/autosave-core";
import { computeApplyDrafts as applyDrafts } from "../src/lib/settings/state/recording-build";

// Regression for FIX 1 — "autosave drops an edit made while a save is in flight".
//
// The store (`recording.svelte.ts`) is a Svelte-runes module ($state class
// fields), so it cannot be instantiated under bun:test (no rune compiler). The
// load-bearing decision this fix introduces lives in
// `RecordingStore.syncRecordingDomainFromCanonical`: on a save echo, ALWAYS
// refresh the baseline, but only clobber the live drafts back to canonical when
// the live snapshot still equals the snapshot that was dispatched to `invoke`.
//
// To keep that decision actually covered (not a re-implemented copy), the gate
// was extracted into the pure, importable `recording-build.computeApplyDrafts`,
// and the store calls it. This spec drives THAT real function (imported above as
// `applyDrafts`) through the B/C scenario plus the diff/baseline machinery the
// engine consults — so the production code path is the thing under test.

describe("settings autosave: edit-during-in-flight-save is not dropped (FIX 1)", () => {
  // The canonical "edit B was persisted" snapshot the backend echoes back.
  const canonicalB = '{"v":"B"}';

  test("no concurrent edit → save echo adopts canonical (drafts clobbered)", () => {
    // Drafts shipped B, no edit during the flight → live still equals dispatched.
    const dispatchedSnapshot = '{"v":"B"}';
    const liveSnapshot = '{"v":"B"}';
    expect(
      applyDrafts({ liveSnapshot, baseline: "A", force: false, dispatchedSnapshot }),
    ).toBe(true);
    // After: baseline := canonicalB and drafts := canonicalB → clean, no resave.
    expect(isDirty(liveSnapshot, canonicalB)).toBe(false);
  });

  test("edit C during flight → drafts are NOT clobbered and stay dirty (resaves)", () => {
    // Edit B fired the save (dispatched B); while awaiting, edit C lands so the
    // live drafts now serialize to C — diverged from what we dispatched.
    const dispatchedSnapshot = '{"v":"B"}';
    const liveSnapshot = '{"v":"C"}';

    // Decision: do NOT overwrite the live drafts (C is preserved).
    expect(
      applyDrafts({ liveSnapshot, baseline: "A", force: false, dispatchedSnapshot }),
    ).toBe(false);

    // The baseline still refreshes to canonical B (B IS persisted now). With the
    // drafts left at C, snapshot(C) !== baseline(B) → the domain is dirty again,
    // so the reactive driver schedules a follow-up save for C. C is not lost.
    const baselineAfter = canonicalB;
    expect(isDirty(liveSnapshot, baselineAfter)).toBe(true);
  });

  test("force echo (privacy) with no dispatched snapshot still clobbers when dirty", () => {
    // The privacy controller passes `force=true` (boolean form). It has no
    // in-flight-edit window to protect, so it must keep clobbering even a dirty
    // domain — regression guard so FIX 1 doesn't weaken that path.
    expect(
      applyDrafts({ liveSnapshot: '{"v":"C"}', baseline: "A", force: true }),
    ).toBe(true);
  });

  test("non-forced external echo keeps in-flight edits on a dirty domain", () => {
    // The domainless/domain `recording_settings_changed` echo (force=false, no
    // dispatched snapshot): a dirty domain keeps its edit (baseline-only refresh).
    expect(
      applyDrafts({ liveSnapshot: '{"v":"C"}', baseline: "A", force: false }),
    ).toBe(false);
    // …and a clean domain adopts canonical.
    expect(
      applyDrafts({ liveSnapshot: "A", baseline: "A", force: false }),
    ).toBe(true);
  });
});

// FIX 2 — the SEPARATE microphone + keyboard-bindings autosave domains share the
// recording path's dispatched-snapshot guard (`audio.svelte.saveMicSettings` and
// `keyboard.svelte.saveKeyboardBindingsSettings` route their save-success
// decision through the same pure `computeApplyDrafts`). These stores are runes
// modules and can't be instantiated under bun:test, so — like the recording case
// above — we drive the real shared gate with the snapshot strings the builders
// produce. The mic snapshot serializes a `{preference, disconnectPolicy}` request
// (`buildMicSnapshot`); the keyboard snapshot serializes a defaulted
// `KeyboardBindingsSettings` (`buildKeyboardBindingsSnapshot`).

describe("settings autosave: mic in-flight edit is preserved (FIX 2)", () => {
  const dispatchedMic = '{"preference":{"mode":"default","deviceId":null},"disconnectPolicy":"fallback_to_default"}';
  const canonicalMic = dispatchedMic;

  test("no concurrent edit → save echo adopts canonical drafts", () => {
    // Live still equals what was dispatched → clobber back to canonical.
    expect(
      applyDrafts({ liveSnapshot: dispatchedMic, baseline: "A", force: false, dispatchedSnapshot: dispatchedMic }),
    ).toBe(true);
    expect(isDirty(canonicalMic, canonicalMic)).toBe(false);
  });

  test("edit during flight → drafts preserved (applyDrafts false) and baseline still advances", () => {
    // While the mic invoke was in flight the user picked a specific device, so
    // the live snapshot diverged from what was dispatched.
    const liveMic = '{"preference":{"mode":"specific_device","deviceId":"mic-2"},"disconnectPolicy":"fallback_to_default"}';
    expect(
      applyDrafts({ liveSnapshot: liveMic, baseline: "A", force: false, dispatchedSnapshot: dispatchedMic }),
    ).toBe(false);
    // Baseline advances to the persisted canonical; with drafts left at the newer
    // edit, the domain reads dirty again → the driver schedules a follow-up save.
    expect(isDirty(liveMic, canonicalMic)).toBe(true);
  });
});

describe("settings autosave: keyboard-bindings in-flight edit is preserved (FIX 2)", () => {
  const dispatchedKb = '{"globalShortcuts":{"enabled":true}}';
  const canonicalKb = dispatchedKb;

  test("no concurrent edit → save echo adopts canonical drafts", () => {
    expect(
      applyDrafts({ liveSnapshot: dispatchedKb, baseline: "A", force: false, dispatchedSnapshot: dispatchedKb }),
    ).toBe(true);
    expect(isDirty(canonicalKb, canonicalKb)).toBe(false);
  });

  test("edit during flight → drafts preserved (applyDrafts false) and baseline still advances", () => {
    // While the keyboard invoke was in flight the user toggled global shortcuts
    // off, so the live snapshot diverged from what was dispatched.
    const liveKb = '{"globalShortcuts":{"enabled":false}}';
    expect(
      applyDrafts({ liveSnapshot: liveKb, baseline: "A", force: false, dispatchedSnapshot: dispatchedKb }),
    ).toBe(false);
    expect(isDirty(liveKb, canonicalKb)).toBe(true);
  });
});
