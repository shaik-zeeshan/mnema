import { describe, expect, test } from "bun:test";
import {
  RECORDING_AUTOSAVE_DOMAINS,
  RECORDING_DRAFT_DOMAINS,
  RECORDING_DOMAIN_COMMANDS,
  RECORDING_AUTOSAVE_DEBOUNCE_MS,
  MIC_AUTOSAVE_DEBOUNCE_MS,
  makeRecordingDomainState,
  isRecordingDraftDomain,
  domainCommand,
  isDirty,
  shouldSaveDomain,
  type AutosaveRecordingDomain,
} from "../src/lib/settings/state/autosave-core";

describe("autosave-core: domain lists + command mapping", () => {
  test("draft domains = autosave domains plus app_privacy_exclusion", () => {
    expect(RECORDING_DRAFT_DOMAINS).toEqual([
      ...RECORDING_AUTOSAVE_DOMAINS,
      "app_privacy_exclusion",
    ]);
    // app_privacy_exclusion is a draft domain but NOT autosaved.
    expect(RECORDING_AUTOSAVE_DOMAINS).not.toContain("app_privacy_exclusion" as AutosaveRecordingDomain);
  });

  test("every autosave domain maps to exactly one update command", () => {
    for (const domain of RECORDING_AUTOSAVE_DOMAINS) {
      expect(typeof RECORDING_DOMAIN_COMMANDS[domain]).toBe("string");
      expect(RECORDING_DOMAIN_COMMANDS[domain].length).toBeGreaterThan(0);
      expect(domainCommand(domain)).toBe(RECORDING_DOMAIN_COMMANDS[domain]);
    }
    // The command map covers exactly the autosave domains — no extras, no gaps.
    expect(Object.keys(RECORDING_DOMAIN_COMMANDS).sort()).toEqual(
      [...RECORDING_AUTOSAVE_DOMAINS].sort(),
    );
  });

  test("known command mappings are stable (wire contract with the backend)", () => {
    expect(RECORDING_DOMAIN_COMMANDS.capture_sources).toBe("update_capture_source_settings");
    expect(RECORDING_DOMAIN_COMMANDS.ai_runtime).toBe("update_ai_runtime_settings");
    expect(RECORDING_DOMAIN_COMMANDS.user_context).toBe("update_user_context_settings");
    expect(RECORDING_DOMAIN_COMMANDS.processing).toBe("update_processing_settings");
  });

  test("debounce windows", () => {
    expect(RECORDING_AUTOSAVE_DEBOUNCE_MS).toBe(450);
    expect(MIC_AUTOSAVE_DEBOUNCE_MS).toBe(250);
  });

  test("isRecordingDraftDomain narrows known domains", () => {
    expect(isRecordingDraftDomain("video")).toBe(true);
    expect(isRecordingDraftDomain("app_privacy_exclusion")).toBe(true);
    expect(isRecordingDraftDomain("not_a_domain")).toBe(false);
  });

  test("makeRecordingDomainState seeds every draft domain", () => {
    const state = makeRecordingDomainState(false);
    expect(Object.keys(state).sort()).toEqual([...RECORDING_DRAFT_DOMAINS].sort());
    for (const domain of RECORDING_DRAFT_DOMAINS) expect(state[domain]).toBe(false);
  });
});

describe("autosave-core: snapshot/diff equality", () => {
  test("a null baseline is never dirty (unloaded)", () => {
    expect(isDirty('{"a":1}', null)).toBe(false);
  });

  test("equal snapshot + baseline is clean", () => {
    expect(isDirty('{"a":1}', '{"a":1}')).toBe(false);
  });

  test("diverging snapshot is dirty", () => {
    expect(isDirty('{"a":2}', '{"a":1}')).toBe(true);
  });
});

describe("autosave-core: save-block gating", () => {
  const dirty = { current: '{"a":2}', baseline: '{"a":1}' };

  test("a clean, unblocked, idle domain does not save", () => {
    expect(
      shouldSaveDomain({ current: '{"a":1}', baseline: '{"a":1}', blocked: false, privacyCommandInFlight: false, saving: false }),
    ).toBe(false);
  });

  test("a dirty, unblocked, idle domain saves", () => {
    expect(
      shouldSaveDomain({ ...dirty, blocked: false, privacyCommandInFlight: false, saving: false }),
    ).toBe(true);
  });

  test("validation block suppresses a dirty domain", () => {
    expect(
      shouldSaveDomain({ ...dirty, blocked: true, privacyCommandInFlight: false, saving: false }),
    ).toBe(false);
  });

  test("an in-flight privacy command suppresses a dirty domain", () => {
    expect(
      shouldSaveDomain({ ...dirty, blocked: false, privacyCommandInFlight: true, saving: false }),
    ).toBe(false);
  });

  test("an already-saving domain does not double-save", () => {
    expect(
      shouldSaveDomain({ ...dirty, blocked: false, privacyCommandInFlight: false, saving: true }),
    ).toBe(false);
  });

  test("a null baseline is never saved regardless of other inputs", () => {
    expect(
      shouldSaveDomain({ current: '{"a":2}', baseline: null, blocked: false, privacyCommandInFlight: false, saving: false }),
    ).toBe(false);
  });
});
