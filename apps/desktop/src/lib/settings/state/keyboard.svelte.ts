// Keyboard-shortcuts (Shortcuts) settings store — Slice-5 shell-ification.
//
// The keyboard bindings are their OWN autosave domain (the `keyboard_bindings`
// engine unit, command `update_keyboard_bindings_settings`), separate from the
// recording-settings domains. `draftGlobalShortcutsEnabled` belongs here (it is
// part of `buildKeyboardBindingsRequest`). This module owns the draft, the
// shortcut-capture state machine, validation, and the autosave baseline so the
// Shortcuts panel is self-contained. Behavior is a 1:1 port of the page-local
// keyboard code it replaces.

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { humanizeError } from "$lib/format-error";
import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";
import {
  DEFAULT_KEYBOARD_BINDINGS,
  EDITABLE_SHORTCUT_ACTIONS,
  getShortcutBinding,
  normalizeShortcutBinding,
  parseShortcutBinding,
  reservedShortcutConflict,
  setShortcutBinding,
  shortcutBindingFromKeyboardEvent,
  shortcutConflictScope,
  shortcutScopesConflict,
  withKeyboardBindingDefaults,
  type EditableShortcutAction,
  type EditableShortcutActionId,
} from "$lib/keyboard-bindings.svelte";
import { RECORDING_AUTOSAVE_DEBOUNCE_MS } from "./autosave-core";
import { computeApplyDrafts } from "./recording-build";
import type { AutosaveEngine } from "./autosave.svelte";
import type { KeyboardBindingsSettings } from "$lib/types";

export class KeyboardStore {
  keyboardBindingsSettings = $state<KeyboardBindingsSettings | null>(null);
  draftGlobalShortcutsEnabled = $state(true);

  savingKeyboardBindings = $state(false);
  keyboardBindingsError = $state<string | null>(null);
  keyboardBindingsSaved = $state(false);

  lastSavedKeyboardBindingsSnapshot = $state<string | null>(null);

  shortcutCaptureActionId = $state<EditableShortcutActionId | null>(null);
  shortcutCaptureError = $state<{ actionId: EditableShortcutActionId; message: string } | null>(null);

  // Global shortcuts the OS refused to register — another app already owns the
  // combination. macOS has no API to say WHICH app, so the row never guesses a
  // name (round-4 decision G9). This is a report on an already-saved binding,
  // so it never blocks autosave the way `shortcutIssues()` does.
  registrationFailures = $state<EditableShortcutActionId[]>([]);

  readonly keyboardPlatform = detectKeyboardPlatform();

  // ─── Build / snapshot / sync ────────────────────────────────────────────────
  buildKeyboardBindingsRequest(): KeyboardBindingsSettings {
    const current = withKeyboardBindingDefaults(this.keyboardBindingsSettings ?? DEFAULT_KEYBOARD_BINDINGS);
    return {
      ...current,
      globalShortcuts: {
        ...current.globalShortcuts,
        enabled: this.draftGlobalShortcutsEnabled,
      },
    };
  }

  buildKeyboardBindingsSnapshot(): string {
    return JSON.stringify(this.buildKeyboardBindingsRequest());
  }

  // The persisted-baseline snapshot for a canonical settings value (defaults
  // applied, like a fresh sync). Used to advance the baseline without clobbering
  // live drafts when an edit lands mid-flight.
  buildSnapshotFromKeyboardBindings(s: KeyboardBindingsSettings): string {
    return JSON.stringify(withKeyboardBindingDefaults(s));
  }

  syncKeyboardBindingsDrafts(s: KeyboardBindingsSettings) {
    this.keyboardBindingsSettings = withKeyboardBindingDefaults(s);
    this.draftGlobalShortcutsEnabled = this.keyboardBindingsSettings.globalShortcuts.enabled;
    this.lastSavedKeyboardBindingsSnapshot = this.buildKeyboardBindingsSnapshot();
  }

  // ─── Shortcut helpers ────────────────────────────────────────────────────────
  shortcutCategoryLabel(category: string): string {
    if (category === "global") return "Recording & window";
    if (category === "app") return "App";
    if (category === "dashboard") return "Dashboard";
    return "Audio Drawer";
  }

  shortcutCategoryActions(category: string): EditableShortcutAction[] {
    return EDITABLE_SHORTCUT_ACTIONS.filter((action) => action.category === category);
  }

  shortcutDraftBinding(actionId: EditableShortcutActionId): string {
    return getShortcutBinding(this.buildKeyboardBindingsRequest(), actionId);
  }

  #bindingHasNonShiftModifier(binding: string): boolean {
    const parsed = parseShortcutBinding(binding);
    return parsed?.primary === true || parsed?.alt === true;
  }

  shortcutIssues(): Record<string, string> {
    const settings = this.buildKeyboardBindingsRequest();
    const issues: Record<string, string> = {};
    const seen = new Map<string, EditableShortcutAction[]>();

    for (const action of EDITABLE_SHORTCUT_ACTIONS) {
      const raw = getShortcutBinding(settings, action.id).trim();
      if (!raw) continue;
      const normalized = normalizeShortcutBinding(raw);
      if (!normalized) {
        issues[action.id] = "Use a valid shortcut such as J, ⌘K, or ⌥⌘P.";
        continue;
      }
      if (action.nativeBackground && !this.#bindingHasNonShiftModifier(normalized)) {
        issues[action.id] = "Background shortcuts must include Command/Control or Alt.";
        continue;
      }
      const reserved = reservedShortcutConflict(action, normalized);
      if (reserved) {
        issues[action.id] = `Reserved to ${reserved.label}.`;
        continue;
      }
      const key = normalized.toLowerCase();
      const previousActions = seen.get(key) ?? [];
      const conflictingPreviousActions = previousActions.filter((previous) =>
        shortcutScopesConflict(shortcutConflictScope(previous), shortcutConflictScope(action)),
      );
      if (conflictingPreviousActions.length > 0) {
        // Name the owner AND the combination (G9) — "already used by X" alone
        // makes the user hunt for which row X is.
        const combo = this.shortcutComboLabel(normalized);
        issues[action.id] = `Already used by ${conflictingPreviousActions[0].label} — ${combo}.`;
        for (const previous of conflictingPreviousActions) {
          issues[previous.id] = `Already used by ${action.label} — ${combo}.`;
        }
      }
      previousActions.push(action);
      seen.set(key, previousActions);
    }

    return issues;
  }

  keyboardShortcutIssues = $derived(this.shortcutIssues());
  keyboardShortcutSaveBlocked = $derived(
    Object.keys(this.keyboardShortcutIssues).length > 0 || this.shortcutCaptureActionId !== null,
  );

  shortcutIssueFor(actionId: EditableShortcutActionId): string | null {
    if (this.shortcutCaptureError?.actionId === actionId) return this.shortcutCaptureError.message;
    const issue = this.keyboardShortcutIssues[actionId];
    if (issue) return issue;
    if (this.registrationFailures.includes(actionId)) {
      return "This shortcut is taken by another app — try a different combination.";
    }
    return null;
  }

  setShortcutDraft(actionId: EditableShortcutActionId, binding: string): void {
    const base = withKeyboardBindingDefaults(this.keyboardBindingsSettings ?? DEFAULT_KEYBOARD_BINDINGS);
    this.keyboardBindingsSettings = setShortcutBinding(base, actionId, binding);
    // The old failure described the old combination; the next save re-registers
    // and reports afresh.
    this.registrationFailures = this.registrationFailures.filter((id) => id !== actionId);
  }

  clearShortcut(actionId: EditableShortcutActionId): void {
    this.setShortcutDraft(actionId, "");
  }

  resetShortcut(actionId: EditableShortcutActionId): void {
    this.setShortcutDraft(actionId, getShortcutBinding(DEFAULT_KEYBOARD_BINDINGS, actionId));
  }

  async restoreDefaultShortcuts(): Promise<void> {
    const ok = await ask("Restore all keyboard shortcuts to their defaults?", {
      title: "Restore default shortcuts",
      kind: "warning",
      okLabel: "Restore defaults",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    this.keyboardBindingsSettings = withKeyboardBindingDefaults(DEFAULT_KEYBOARD_BINDINGS);
    this.draftGlobalShortcutsEnabled = DEFAULT_KEYBOARD_BINDINGS.globalShortcuts.enabled;
  }

  // "⌘⌥Space" on macOS, "Ctrl+Alt+Space" elsewhere — the same tokens the row's
  // <kbd> caps render, flattened into a sentence.
  shortcutComboLabel(binding: string): string {
    const tokens = this.shortcutKeyTokens(binding);
    if (!tokens) return binding;
    return tokens.join(this.keyboardPlatform === "macos" ? "" : "+");
  }

  shortcutKeyTokens(binding: string): string[] | null {
    const parsed = parseShortcutBinding(binding);
    if (!parsed) return null;
    return formatShortcut(parsed, this.keyboardPlatform);
  }

  startShortcutCapture(actionId: EditableShortcutActionId): void {
    this.shortcutCaptureError = null;
    this.shortcutCaptureActionId = this.shortcutCaptureActionId === actionId ? null : actionId;
  }

  cancelShortcutCapture(): void {
    this.shortcutCaptureError = null;
    this.shortcutCaptureActionId = null;
  }

  captureShortcut(actionId: EditableShortcutActionId, event: KeyboardEvent): void {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
    if (event.key === "Escape") {
      this.shortcutCaptureError = null;
      this.shortcutCaptureActionId = null;
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      this.shortcutCaptureError = null;
      this.clearShortcut(actionId);
      this.shortcutCaptureActionId = null;
      return;
    }
    const binding = shortcutBindingFromKeyboardEvent(event, this.keyboardPlatform);
    if (!binding) {
      if (this.keyboardPlatform === "macos" && event.ctrlKey && event.key !== "Control") {
        this.shortcutCaptureError = { actionId, message: "Control shortcuts are not supported on macOS. Use Command or Option." };
      } else if (event.key !== "Meta" && event.key !== "Control" && event.key !== "Alt" && event.key !== "Shift") {
        this.shortcutCaptureError = { actionId, message: "That key is not supported for shortcuts." };
      }
      return;
    }
    this.shortcutCaptureError = null;
    this.setShortcutDraft(actionId, binding);
    this.shortcutCaptureActionId = null;
  }

  // ─── Load / save ────────────────────────────────────────────────────────────
  async loadKeyboardBindingsSettings() {
    this.keyboardBindingsError = null;
    try {
      const s = await invoke<KeyboardBindingsSettings>("get_keyboard_bindings_settings");
      this.syncKeyboardBindingsDrafts(s);
    } catch (err) {
      this.keyboardBindingsError = humanizeError(err);
    }
    await this.refreshRegistrationFailures();
  }

  async refreshRegistrationFailures() {
    try {
      this.registrationFailures = await invoke<EditableShortcutActionId[]>(
        "get_global_shortcut_registration_failures",
      );
    } catch {
      // A failure to read the failures is not itself a settings error — the row
      // just stays quiet.
      this.registrationFailures = [];
    }
  }

  async saveKeyboardBindingsSettings() {
    this.savingKeyboardBindings = true;
    this.keyboardBindingsError = null;
    this.keyboardBindingsSaved = false;
    // Snapshot what we are dispatching so we can detect an edit that lands while
    // the invoke is in flight (mirrors the recording path's dispatched-snapshot
    // guard — see `recording-build.computeApplyDrafts`).
    const dispatchedSnapshot = this.buildKeyboardBindingsSnapshot();
    try {
      const updated = await invoke<KeyboardBindingsSettings>("update_keyboard_bindings_settings", {
        request: this.buildKeyboardBindingsRequest(),
      });
      // Adopt canonical drafts only when the live drafts STILL equal what we
      // dispatched (no edit landed during the flight). Otherwise leave the newer
      // drafts alone and only advance the baseline, so the reactive driver
      // schedules a follow-up save for the in-flight edit (it is never dropped).
      const applyDrafts = computeApplyDrafts({
        liveSnapshot: this.buildKeyboardBindingsSnapshot(),
        baseline: this.lastSavedKeyboardBindingsSnapshot,
        force: false,
        dispatchedSnapshot,
      });
      if (applyDrafts) {
        this.keyboardBindingsSettings = updated;
        this.syncKeyboardBindingsDrafts(updated);
      } else {
        this.lastSavedKeyboardBindingsSnapshot = this.buildSnapshotFromKeyboardBindings(updated);
      }
      this.keyboardBindingsSaved = true;
      setTimeout(() => { this.keyboardBindingsSaved = false; }, 2200);
    } catch (err) {
      this.keyboardBindingsError = humanizeError(err);
    } finally {
      this.savingKeyboardBindings = false;
    }
    // The save just re-registered the global shortcuts; pick up whatever the OS
    // refused so the offending row can say so.
    await this.refreshRegistrationFailures();
  }

  // ─── Autosave registration ──────────────────────────────────────────────────
  registerAutosave(engine: AutosaveEngine) {
    engine.register({
      key: "keyboard_bindings",
      debounceMs: RECORDING_AUTOSAVE_DEBOUNCE_MS,
      snapshot: () => this.buildKeyboardBindingsSnapshot(),
      baseline: () => this.lastSavedKeyboardBindingsSnapshot,
      blocked: () => this.keyboardShortcutSaveBlocked,
      saving: () => this.savingKeyboardBindings,
      save: () => this.saveKeyboardBindingsSettings(),
    });
  }
}

export function createKeyboardStore(): KeyboardStore {
  return new KeyboardStore();
}
