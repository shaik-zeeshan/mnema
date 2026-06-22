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
        issues[action.id] = `Conflicts with ${conflictingPreviousActions[0].label}.`;
        for (const previous of conflictingPreviousActions) {
          issues[previous.id] = `Conflicts with ${action.label}.`;
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
    return this.keyboardShortcutIssues[actionId] ?? null;
  }

  setShortcutDraft(actionId: EditableShortcutActionId, binding: string): void {
    const base = withKeyboardBindingDefaults(this.keyboardBindingsSettings ?? DEFAULT_KEYBOARD_BINDINGS);
    this.keyboardBindingsSettings = setShortcutBinding(base, actionId, binding);
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
      this.keyboardBindingsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    }
  }

  async saveKeyboardBindingsSettings() {
    this.savingKeyboardBindings = true;
    this.keyboardBindingsError = null;
    this.keyboardBindingsSaved = false;
    try {
      const updated = await invoke<KeyboardBindingsSettings>("update_keyboard_bindings_settings", {
        request: this.buildKeyboardBindingsRequest(),
      });
      this.keyboardBindingsSettings = updated;
      this.syncKeyboardBindingsDrafts(updated);
      this.keyboardBindingsSaved = true;
      setTimeout(() => { this.keyboardBindingsSaved = false; }, 2200);
    } catch (err) {
      this.keyboardBindingsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      this.savingKeyboardBindings = false;
    }
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
