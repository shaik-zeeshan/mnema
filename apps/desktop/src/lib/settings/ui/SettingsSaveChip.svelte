<script lang="ts">
  // The one settings save-state surface (DECISIONS.md G7).
  //
  // "No bottom save bar, ever": this chip sits in a top-anchored strip ABOVE the
  // scroll region, so it can never clip off the bottom of a short window. It is
  // the whole at-rest story — idle / saving / saved / blocked / failed — and it
  // owns the failure toasts too, so the persistent chip state and the toast that
  // carries the message + Retry can never disagree.
  //
  // The row-level "Saved ✓" echo (which row saved) is driven from here: entering
  // the "ok" state calls `noteSaved()`, and `SettingRow` renders the echo on
  // whichever row the user last touched.

  import IconCheck from "~icons/lucide/check";
  import IconWarn from "~icons/lucide/triangle-alert";
  import { dismissToast, toast } from "$lib/toast.svelte";
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

  // Entering "ok" is the one success signal every autosaved surface shares.
  $effect(() => {
    if (status === "ok") noteSaved();
  });

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

<div class="save-strip">
  <span class="savechip savechip--{status}" role="status" aria-live="polite">
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
</div>

<style>
  /* Top-anchored, outside the scroll region — the chip's whole point is that it
     cannot clip. Right-aligned so it reads as chrome, not as content. */
  .save-strip {
    flex: 0 0 auto;
    display: flex;
    justify-content: flex-end;
    align-items: center;
    min-height: 24px;
  }

  .savechip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    background: var(--app-surface);
    box-shadow: inset 0 0 0 1px var(--app-border);
    font-size: var(--t-meta);
    color: var(--app-text-muted);
    white-space: nowrap;
    transition: color 0.15s ease, background 0.15s ease;
  }

  .savechip :global(svg) {
    width: 11px;
    height: 11px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .savechip--ok {
    color: var(--app-accent);
    background: color-mix(in srgb, var(--app-accent) 10%, var(--app-surface));
  }
  .savechip--saving {
    color: var(--app-accent);
  }
  .savechip--blocked {
    color: var(--app-warn-strong);
  }
  .savechip--error {
    color: var(--app-danger-text);
    background: color-mix(in srgb, var(--app-danger) 12%, var(--app-surface));
    box-shadow: inset 0 0 0 1px var(--app-danger);
  }

  .savechip__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
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
