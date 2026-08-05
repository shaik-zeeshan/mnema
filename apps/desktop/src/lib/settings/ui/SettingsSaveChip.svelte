<script lang="ts">
  // The one settings save-state surface (DECISIONS.md G7).
  //
  // "No bottom save bar, ever": this chip rides in the 30px tool strip ABOVE the
  // scroll region, so it can never clip off a short window, and the same state
  // is published to the window-edge status strip (direction 02's "whether" half
  // of autosave). It is the whole at-rest story — idle / saving / saved /
  // blocked / failed — and it owns the failure toasts too, so the persistent
  // chip state and the toast that carries the message + Retry can never
  // disagree.
  //
  // NOT here, deliberately: an Undo next to the chip. Both mockups draw one;
  // G7 rules Settings-Undo OUT for v1 — the chip plus the row echo answer the
  // anxiety, and an undo stack is speculative machinery.
  //
  // The row-level "Saved ✓" echo (which row saved) is driven from here: entering
  // the "ok" state calls `noteSaved()`, and `SettingRow` renders the echo on
  // whichever row the user last touched.

  import IconCheck from "~icons/lucide/check";
  import IconWarn from "~icons/lucide/triangle-alert";
  import { dismissToast, toast } from "$lib/toast.svelte";
  import { getSettingsController } from "../state/controller.svelte";
  import { noteSaved } from "../state/row-echo.svelte";
  import { statusSave } from "$lib/studio/status-strip.svelte";

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

  // Entering "ok" is the one success signal every autosaved surface shares.
  $effect(() => {
    if (status === "ok") noteSaved();
  });

  // Direction 02 puts the "whether" half of autosave in the status strip, which
  // the root layout owns. Publish this chip's state there while Settings is
  // mounted, and clear it on the way out so the strip never shows a stale save
  // state on Timeline. Verified by driving a real save and watching the chip and
  // the strip step through saving \u2192 saved together.
  $effect(() => {
    const s = status;
    statusSave.set({
      tone: s === "error" || s === "blocked" ? "bad" : s === "saving" ? "busy" : "ok",
      label:
        s === "error"
          ? "Not saved"
          : s === "blocked"
            ? "Resolve issues to save"
            : s === "saving"
              ? "Saving\u2026"
              : s === "ok"
                ? "Saved"
                : "All changes saved",
    });
  });

  // Clearing is an unmount concern only. Putting it in the publish effect's
  // teardown makes every re-publish a null-then-value pair for no reason.
  $effect(() => () => statusSave.set(null));

  // Autosave failures are the one persistent settings state, and they also raise
  // a toast that never auto-dismisses (G7). The stable ids replace rather than
  // stack, so a hammering retry can't pile up rows; dismissing clears the same
  // state the chip reads, so the chip can never outlive its message.
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

<span
  class="ss-save savechip"
  class:ss-save--busy={status === "saving"}
  class:ss-save--bad={status === "error" || status === "blocked"}
  role="status"
  aria-live="polite"
>
  {#if status === "error"}
    <IconWarn aria-hidden="true" />
    Not saved
  {:else if status === "blocked"}
    <IconWarn aria-hidden="true" />
    Resolve issues to save
  {:else if status === "saving"}
    <span class="savechip__dot" aria-hidden="true"></span>
    Saving…
  {:else if status === "ok"}
    <IconCheck aria-hidden="true" />
    Saved
  {:else}
    <IconCheck aria-hidden="true" />
    All changes saved
  {/if}
</span>

<style>
  /* The chip's shape is the skin's `.ss-save`, so the strip and this one read as
     the same object in two places. Only the icon sizing + the saving pulse are
     local. */
  .savechip {
    flex: 0 0 auto;
  }

  .savechip :global(svg) {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .savechip__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    animation: chip-pulse 1.1s ease-in-out infinite;
  }

  @keyframes chip-pulse {
    0%, 100% { opacity: 0.35; transform: scale(0.85); }
    50% { opacity: 1; transform: scale(1.1); }
  }

  @media (prefers-reduced-motion: reduce) {
    .savechip__dot {
      animation: none;
    }
  }
</style>
