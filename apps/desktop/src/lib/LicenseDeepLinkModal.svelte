<script lang="ts">
  // License deep-link receipt — the one visible acknowledgement that a
  // mnema://license/* deep link brought the user back to the app. Opens
  // instantly in its "working" face on the `license_deep_link` event (emitted
  // by the Rust dispatcher before any handler work; a cold-start window takes
  // the queued announcement instead), then morphs in place as results land on
  // the existing `license_status` channel. Face policy lives in
  // `$lib/license-deeplink-receipt.ts`; this component only renders.
  //
  // Chrome (overlay/panel, backdrop-pointerdown-to-close, Escape, Tab-trap,
  // opener focus handoff) mirrors FrameDetailModal.

  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { trapTabKey } from "$lib/keyboard";
  import { licenseStatus } from "$lib/licensing-store.svelte";
  import { safeExternalUrl } from "$lib/licensing-panel";
  import { openSettings } from "$lib/surface-windows";
  import {
    fmtReceiptDate,
    receiptFaceFor,
    type DeepLinkFlow,
    type LicenseDeepLinkDone,
    type ReceiptFace,
  } from "$lib/license-deeplink-receipt";
  import type { LicenseStatus } from "$lib/licensing";

  // After this long without a result, soften the working copy — the claim
  // poll can take ~30s and the renewal refresh up to ~60s; never spin mute.
  const SLOW_AFTER_MS = 30_000;

  let open = $state(false);
  let face = $state<ReceiptFace>({ face: "working" });
  let slow = $state(false);

  let flow: DeepLinkFlow = "activate";
  let baseline: LicenseStatus | null = null;
  let baselineRevision = 0;
  // Stays true after a mid-flight dismissal so the episode keeps listening
  // silently: success stays quiet, only the actionable device-limit failure
  // re-opens the modal.
  let episodeActive = false;
  let slowTimer: ReturnType<typeof setTimeout> | null = null;

  function clearSlowTimer(): void {
    if (slowTimer !== null) {
      clearTimeout(slowTimer);
      slowTimer = null;
    }
  }

  function startEpisode(nextFlow: DeepLinkFlow): void {
    flow = nextFlow;
    baseline = licenseStatus.value;
    baselineRevision = licenseStatus.revision;
    face = { face: "working" };
    slow = false;
    episodeActive = true;
    open = true;
    clearSlowTimer();
    slowTimer = setTimeout(() => {
      slow = true;
    }, SLOW_AFTER_MS);
  }

  $effect(() => {
    const unlistenPromise = listen<{ flow: DeepLinkFlow }>("license_deep_link", (event) => {
      startEpisode(event.payload.flow);
    });
    // Terminal endings that never emit a license_status (rejected key, renewal
    // poll timeout, declined replacement, claim's native email-dialog handoff).
    const unlistenDonePromise = listen<LicenseDeepLinkDone>("license_deep_link_done", (event) => {
      if (!episodeActive) return;
      clearSlowTimer();
      if (event.payload.outcome === "closed") {
        open = false;
        episodeActive = false;
        return;
      }
      face = { face: "failed", message: event.payload.message };
      open = true; // an actionable failure beats silence, even after a dismissal
    });
    // Cold start: the deep link launched the app before this listener existed,
    // so the dispatcher queued the announcement (taking clears the slot).
    void invoke<{ flow: DeepLinkFlow } | null>("take_pending_license_deep_link")
      .then((pending) => {
        if (pending) startEpisode(pending.flow);
      })
      .catch(() => {
        // Non-main surfaces / dev harness without the command: no receipt.
      });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
      void unlistenDonePromise.then((unlisten) => unlisten());
      clearSlowTimer();
    };
  });

  // Morph in place as results land. Only emits AFTER the deep link count — the
  // baseline revision fences off the boot snapshot and anything earlier.
  $effect(() => {
    const revision = licenseStatus.revision;
    const current = licenseStatus.value;
    if (!episodeActive || revision === baselineRevision) return;
    const next = receiptFaceFor(flow, baseline, current);
    if (next.face === "working") return; // result still in flight
    if (open) {
      face = next;
      clearSlowTimer();
    } else if (next.face === "overCap") {
      face = next;
      open = true;
    } else {
      episodeActive = false; // dismissed mid-flight + resolved fine → stay quiet
    }
  });

  function close(): void {
    open = false;
    clearSlowTimer();
    if (face.face !== "working") episodeActive = false;
  }

  function openExternal(url: string): void {
    const safe = safeExternalUrl(url);
    if (!safe) return;
    void openUrl(safe).catch((e) =>
      console.error("[LicenseDeepLink] open external failed", e),
    );
  }

  const ariaLabel = $derived(
    {
      working: "Setting up your license",
      activated: "License activated",
      pending: "License installed, activating",
      renewed: "Update window extended",
      overCap: "Device limit reached",
      failed: "Couldn't finish this license link",
    }[face.face],
  );

  const workingSub = $derived(
    slow
      ? "Still working — you can close this; we'll finish in the background."
      : {
          activate: "Verifying your license key…",
          claim: "Waiting for your order to be confirmed — usually a few seconds.",
          renewed: "Confirming your renewal — usually under a minute.",
        }[flow],
  );

  function days(n: number): string {
    return `${n} ${n === 1 ? "day" : "days"}`;
  }

  // ---- Chrome: backdrop close + focus handoff (mirrors FrameDetailModal) ---
  let panelEl = $state<HTMLDivElement | null>(null);
  let opener: HTMLElement | null = null;
  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      opener = document.activeElement as HTMLElement | null;
      panelEl?.focus();
    } else if (!open && wasOpen) {
      const trigger = opener;
      opener = null;
      void tick().then(() => trigger?.focus());
    }
    wasOpen = open;
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (!open) return;
    if (trapTabKey(e, panelEl)) return;
    if (e.key !== "Escape") return;
    close();
  }}
/>

{#if open}
  <div
    class="license-receipt"
    role="presentation"
    onpointerdown={(e) => {
      if (e.target !== e.currentTarget) return;
      close();
    }}
  >
    <div
      bind:this={panelEl}
      class="license-receipt__panel"
      role="dialog"
      aria-modal="true"
      aria-label={ariaLabel}
      aria-busy={face.face === "working"}
      tabindex="-1"
    >
      {#if face.face === "working"}
        <div class="receipt-glyph receipt-glyph--busy">…</div>
        <h1 class="receipt-title">Setting up your license</h1>
        <p class="receipt-sub">{workingSub}</p>
        <div class="receipt-row">
          <button type="button" class="receipt-btn receipt-btn--ghost" onclick={close}>
            Dismiss
          </button>
        </div>
      {:else if face.face === "activated"}
        <div class="receipt-glyph receipt-glyph--ok">✓</div>
        <h1 class="receipt-title">License activated</h1>
        <p class="receipt-who">Licensed to {face.owner}</p>
        <p class="receipt-sub">Updates included through {fmtReceiptDate(face.updateThroughMs)}</p>
        <div class="receipt-row">
          <button type="button" class="receipt-btn receipt-btn--primary" onclick={close}>
            Continue
          </button>
        </div>
      {:else if face.face === "pending"}
        <div class="receipt-glyph receipt-glyph--pending">◌</div>
        <span class="receipt-badge">Activating…</span>
        <h1 class="receipt-title">License installed</h1>
        <p class="receipt-who">Licensed to {face.owner}</p>
        <p class="receipt-sub">
          Confirming this device in the background — {days(face.provisionalDaysLeft)} to connect.
          Mnema is fully unlocked meanwhile.
        </p>
        <div class="receipt-row">
          <button type="button" class="receipt-btn receipt-btn--primary" onclick={close}>
            Continue
          </button>
        </div>
      {:else if face.face === "renewed"}
        <div class="receipt-glyph receipt-glyph--ok">✓</div>
        <h1 class="receipt-title">Update window extended</h1>
        <p class="receipt-who">Updates included through {fmtReceiptDate(face.updateThroughMs)}</p>
        {#if face.wasMs !== null}
          <p class="receipt-sub"><s class="receipt-was">was {fmtReceiptDate(face.wasMs)}</s></p>
        {:else}
          <p class="receipt-sub">Thanks for renewing.</p>
        {/if}
        <div class="receipt-row">
          <button type="button" class="receipt-btn receipt-btn--primary" onclick={close}>
            Continue
          </button>
        </div>
      {:else if face.face === "overCap"}
        {@const cap = face}
        <div class="receipt-glyph receipt-glyph--warn">!</div>
        <h1 class="receipt-title">Device limit reached</h1>
        <p class="receipt-who">This license is already active on its 3 devices.</p>
        <p class="receipt-sub">Free up a device to activate this Mac, or buy another license.</p>
        <div class="receipt-row">
          <button
            type="button"
            class="receipt-btn receipt-btn--ghost"
            onclick={() => openExternal(cap.buyUrl)}
          >
            Buy another license
          </button>
          <button
            type="button"
            class="receipt-btn receipt-btn--primary"
            onclick={() => openExternal(cap.resetUrl)}
          >
            Free up my devices
          </button>
        </div>
      {:else if face.face === "failed"}
        <div class="receipt-glyph receipt-glyph--warn">!</div>
        <h1 class="receipt-title">Couldn't finish this license link</h1>
        <p class="receipt-sub">{face.message}</p>
        <div class="receipt-row">
          <button type="button" class="receipt-btn receipt-btn--ghost" onclick={close}>
            Close
          </button>
          <button
            type="button"
            class="receipt-btn receipt-btn--primary"
            onclick={() => {
              close();
              void openSettings("license");
            }}
          >
            Open License Settings
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .license-receipt {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--app-overlay-bg);
    backdrop-filter: blur(10px);
  }
  .license-receipt__panel {
    width: min(400px, 100%);
    padding: 26px 24px 22px;
    text-align: center;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface-raised);
    box-shadow: var(--app-shadow-popover);
  }
  .license-receipt__panel:focus {
    outline: none;
  }

  .receipt-glyph {
    width: 44px;
    height: 44px;
    margin: 0 auto 14px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    font-size: 18px;
  }
  .receipt-glyph--ok {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    box-shadow: 0 0 24px var(--app-accent-glow);
  }
  .receipt-glyph--warn {
    color: var(--app-warn);
    background: var(--app-warn-bg);
    border: 1px solid var(--app-warn-border);
  }
  .receipt-glyph--busy {
    position: relative;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border-strong);
  }
  .receipt-glyph--busy::after {
    content: "";
    position: absolute;
    inset: -1px;
    border-radius: 50%;
    border: 1px solid transparent;
    border-top-color: var(--app-accent);
    animation: receipt-spin 1s linear infinite;
  }
  /* Installed-but-unconfirmed: a slow pulse, deliberately not a checkmark. */
  .receipt-glyph--pending {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px dashed var(--app-accent-border);
    animation: receipt-pulse 2.2s ease-in-out infinite;
  }
  @keyframes receipt-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes receipt-pulse {
    50% {
      box-shadow: 0 0 24px var(--app-accent-glow);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .receipt-glyph--busy::after,
    .receipt-glyph--pending {
      animation: none;
    }
  }

  .receipt-badge {
    display: inline-block;
    margin-bottom: 12px;
    padding: 2px 9px;
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .receipt-title {
    margin: 0 0 6px;
    color: var(--app-text-strong);
    font-size: 16px;
    font-weight: 600;
  }
  .receipt-who {
    margin: 0;
    color: var(--app-text);
    font-size: 13px;
  }
  .receipt-sub {
    margin: 2px 0 18px;
    color: var(--app-text-muted);
    font-size: 12px;
    line-height: 1.55;
  }
  .receipt-was {
    color: var(--app-text-faint);
  }

  .receipt-row {
    display: flex;
    gap: 8px;
  }
  .receipt-btn {
    flex: 1;
    padding: 7px 16px;
    border: 1px solid transparent;
    border-radius: 6px;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .receipt-btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .receipt-btn--primary {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
    font-weight: 600;
  }
  .receipt-btn--primary:hover {
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .receipt-btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
  }
  .receipt-btn--ghost:hover {
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }
</style>
