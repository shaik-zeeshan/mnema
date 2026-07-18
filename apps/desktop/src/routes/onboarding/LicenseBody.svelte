<script lang="ts">
  import { licenseStatus } from "$lib/licensing-store.svelte";
  import { statusLineFor } from "$lib/licensing-panel";

  // Purely explanatory — the trial starts at first Capture, not here. This body
  // takes no key and starts nothing; it only sets expectations. Reuses the shared
  // onboarding-body classes (.group/.note) so it matches every other row.

  // Optional live reflection: a returning trial/licensed user re-running
  // onboarding sees their real state instead of the generic pitch. `null` (the
  // gate hasn't run yet, or a genuine first run) → just the explainer below.
  // Copy lives in `licensing-panel.ts` (bun-tested).
  const statusLine = $derived(statusLineFor(licenseStatus.value));
</script>

<div class="group">
  <div class="group-title">How you own Mnema</div>

  <div class="note">
    Mnema is a <b>one-time purchase</b> with a <b>30-day free trial</b> that starts when you first
    record — so every trial day builds real recall history you can actually evaluate. Nothing starts
    here; just press record when you're ready.
  </div>

  {#if statusLine}
    <div class="note muted">{statusLine}</div>
  {/if}

  <div class="note muted">
    When the trial ends, Mnema switches to <b>Read-Only Mode</b>: everything you recorded stays fully
    browsable and searchable — only new recording pauses until you buy. Your history is never locked
    away.
  </div>

  <div class="note muted">
    Buy once, own it. <b>No account, no subscription</b>, and nothing phones home — your license is
    verified entirely on your device.
  </div>
</div>
