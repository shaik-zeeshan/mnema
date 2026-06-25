import { describe, expect, test } from "bun:test";
import { createAutosaveEngine, type AutosaveUnit } from "../src/lib/settings/state/autosave.svelte";
import { RECORDING_DOMAIN_COMMANDS } from "../src/lib/settings/state/autosave-core";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// A controllable fake unit standing in for one autosave domain. Mutate
// `snapshot`/`baseline`/`blocked`/`saving` to drive engine decisions, and read
// `invoked` to see which command "save" ran.
function makeFakeUnit(domain: keyof typeof RECORDING_DOMAIN_COMMANDS, debounceMs = 20) {
  const calls: string[] = [];
  let snapshot = "A";
  let baseline: string | null = "A";
  let blocked = false;
  let saving = false;
  const unit: AutosaveUnit = {
    key: domain,
    debounceMs,
    snapshot: () => snapshot,
    baseline: () => baseline,
    blocked: () => blocked,
    saving: () => saving,
    // The save runner stands in for the real `invoke(RECORDING_DOMAIN_COMMANDS[domain])`.
    save: () => { calls.push(RECORDING_DOMAIN_COMMANDS[domain]); },
  };
  return {
    unit,
    calls,
    setSnapshot: (s: string) => { snapshot = s; },
    setBaseline: (b: string | null) => { baseline = b; },
    setBlocked: (b: boolean) => { blocked = b; },
    setSaving: (b: boolean) => { saving = b; },
  };
}

describe("autosave engine: parity with the per-domain debounce semantics", () => {
  test("a dirty, unblocked domain runs its mapped save after the debounce", async () => {
    const privacy = { inFlight: false };
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => privacy.inFlight });
    const video = makeFakeUnit("video");
    engine.register(video.unit);

    // Make the domain dirty, then tick.
    video.setSnapshot("B"); // baseline is still "A"
    engine.tick();
    expect(engine.hasPendingTimer("video")).toBe(true);
    expect(video.calls).toEqual([]); // not yet — still debouncing

    await sleep(40);
    // The mapped command ran exactly once.
    expect(video.calls).toEqual([RECORDING_DOMAIN_COMMANDS.video]);
    expect(video.calls).toEqual(["update_video_settings"]);
    expect(engine.hasPendingTimer("video")).toBe(false);
  });

  test("a clean domain never schedules a save", async () => {
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
    const storage = makeFakeUnit("storage");
    engine.register(storage.unit);

    // snapshot === baseline → clean.
    engine.tick();
    expect(engine.hasPendingTimer("storage")).toBe(false);
    await sleep(40);
    expect(storage.calls).toEqual([]);
  });

  test("a validation-blocked dirty domain never schedules a save", async () => {
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
    const captureSources = makeFakeUnit("capture_sources");
    engine.register(captureSources.unit);

    captureSources.setSnapshot("B"); // dirty
    captureSources.setBlocked(true); // but blocked
    engine.tick();
    expect(engine.hasPendingTimer("capture_sources")).toBe(false);
    await sleep(40);
    expect(captureSources.calls).toEqual([]);
  });

  test("an in-flight privacy command suppresses every domain's save", async () => {
    const privacy = { inFlight: true };
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => privacy.inFlight });
    const metadata = makeFakeUnit("metadata");
    engine.register(metadata.unit);

    metadata.setSnapshot("B"); // dirty
    engine.tick();
    expect(engine.hasPendingTimer("metadata")).toBe(false);
    await sleep(40);
    expect(metadata.calls).toEqual([]);
  });

  test("a gate that closes during the debounce cancels the save at fire time", async () => {
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
    const display = makeFakeUnit("display");
    engine.register(display.unit);

    display.setSnapshot("B"); // dirty
    engine.tick();
    expect(engine.hasPendingTimer("display")).toBe(true);
    // Block it before the timer fires; the re-check at fire time must abort.
    display.setBlocked(true);
    await sleep(40);
    expect(display.calls).toEqual([]);
  });

  test("cancelAll() drops a pending debounce", async () => {
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
    const inactivity = makeFakeUnit("inactivity");
    engine.register(inactivity.unit);

    inactivity.setSnapshot("B");
    engine.tick();
    expect(engine.hasPendingTimer("inactivity")).toBe(true);
    engine.cancelAll();
    expect(engine.hasPendingTimer("inactivity")).toBe(false);
    await sleep(40);
    expect(inactivity.calls).toEqual([]);
  });

  test("a fresh edit during the window re-arms (debounces) rather than double-firing", async () => {
    const engine = createAutosaveEngine({ privacyCommandInFlight: () => false });
    const developer = makeFakeUnit("developer", 30);
    engine.register(developer.unit);

    developer.setSnapshot("B");
    engine.tick();
    await sleep(15); // mid-window
    developer.setSnapshot("C"); // another edit
    engine.tick(); // re-arms
    await sleep(15); // total 30 from first tick, but only 15 since re-arm → not fired yet
    expect(developer.calls).toEqual([]);
    await sleep(25); // now past the re-armed window
    expect(developer.calls).toEqual([RECORDING_DOMAIN_COMMANDS.developer]); // exactly once
  });
});
