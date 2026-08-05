<script lang="ts">
  // The one settings save-state surface (DECISIONS.md G7) — headless.
  //
  // "No bottom save bar, ever." In direction 04 the persistent, timestamped
  // autosave state lives in the DECK: a 28px bar inside the window frame that
  // never scrolls and therefore cannot clip at any window size. This component
  // renders nothing; it derives the one status and publishes it into the deck's
  // `status` slot, and it owns the failure toasts, so the deck state and the
  // toast that carries the message + Retry can never disagree.
  //
  // The row-level "Saved ✓" echo (which row saved) is driven from here too:
  // entering the "ok" state calls `noteSaved()`, and `SettingRow` renders the
  // echo on whichever row the user last touched.
  //
  // Undo is NOT built (G7: Settings-Undo is out for v1), so no ⌘Z anywhere.

  import { dismissToast, toast } from "$lib/toast.svelte";
  import { setDeck, type DeckStatus } from "$lib/deck.svelte";
  import { getSettingsController } from "../state/controller.svelte";
  import { noteSaved } from "../state/row-echo.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const keyboard = c.keyboard;
  const audio = c.audio;

  const status = $derived<"error" | "blocked" | "saving" | "ok" | "idle">(
    rec.recError || keyboard.keyboardBindingsError || audio.micError
      ? "error"
      : c.recSaveBlocked || audio.micApplyBlocked
        ? "blocked"
        : c.savingRecSettings || keyboard.savingKeyboardBindings || audio.savingMicSettings
          ? "saving"
          : rec.recSaved || keyboard.keyboardBindingsSaved || audio.micSaved
            ? "ok"
            : "idle",
  );

  // What failed, named — the deck slot says the cause, not just "error".
  const failure = $derived(
    rec.recError
      ? "recording settings"
      : keyboard.keyboardBindingsError
        ? "keyboard shortcuts"
        : audio.micError
          ? "microphone settings"
          : null,
  );

  // The timestamp the deck carries. Stamped when a save lands, so it says WHEN
  // rather than just "saved" — the whole reason the state is persistent.
  let savedAt = $state<string | null>(null);

  // Entering "ok" is the one success signal every autosaved surface shares.
  // Writes `savedAt` but never reads it, so this can't self-trigger.
  $effect(() => {
    if (status !== "ok") return;
    noteSaved();
    savedAt = new Date().toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  });

  const deckStatus = $derived<DeckStatus>(
    status === "error"
      ? { tone: "danger", text: `Not saved — ${failure ?? "settings"}` }
      : status === "blocked"
        ? { tone: "danger", text: "Resolve issues to save" }
        : status === "saving"
          ? { tone: "quiet", text: "Saving…" }
          : savedAt
            ? { tone: "ok", text: `All changes saved · ${savedAt}` }
            : { tone: "ok", text: "All changes saved" },
  );

  $effect(() => {
    setDeck({ status: deckStatus });
  });

  // Autosave failures are the one persistent settings state, and they also raise
  // a toast that never auto-dismisses (G7). The stable ids replace rather than
  // stack, so a hammering retry can't pile up rows; dismissing clears the same
  // state the deck reads, so the deck can never outlive its message.
  $effect(() => {
    if (rec.recError) {
      toast({
        id: "settings-save-recording",
        tone: "error",
        title: "Couldn't save recording settings",
        message: rec.recError,
        action: c.lastFailedSaveDomain
          ? { label: "Retry", run: () => c.retryFailedSave() }
          : undefined,
        onDismiss: () => c.dismissRecError(),
      });
    } else {
      dismissToast("settings-save-recording");
    }
  });
  $effect(() => {
    if (keyboard.keyboardBindingsError) {
      toast({
        id: "settings-save-keyboard",
        tone: "error",
        title: "Couldn't save keyboard shortcuts",
        message: keyboard.keyboardBindingsError,
        onDismiss: () => (keyboard.keyboardBindingsError = null),
      });
    } else {
      dismissToast("settings-save-keyboard");
    }
  });
  $effect(() => {
    if (audio.micError) {
      toast({
        id: "settings-save-microphone",
        tone: "error",
        title: "Couldn't save microphone settings",
        message: audio.micError,
        onDismiss: () => (audio.micError = null),
      });
    } else {
      dismissToast("settings-save-microphone");
    }
  });
</script>
