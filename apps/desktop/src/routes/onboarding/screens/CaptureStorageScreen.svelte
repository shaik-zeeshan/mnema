<!--
  Screen 3 / 8 — Capture & Storage. THE ONLY SCREEN IN THE FLOW WITH HARD GATES.

  Two surfaces, no rows: `<CaptureSentence>` (capture rate + retention + folder,
  and every failure/repair those three can produce) and `<ExcludedApps>` (the
  "Never recorded" line, which IS the control). Both live in
  `$lib/onboarding/`; this file is wiring plus the footer.

  CONTRACT
    props
      flow        OnboardingFlow. Reads:
                    flow.downloadBytes         the work-list bytes — what the
                                               downloads will fetch. NOT
                                               `flow.requiredBytes`: the sentence
                                               adds the reserve and a day of
                                               capture itself (`storageNeedBytes`),
                                               so passing the total double-counts.
                    flow.blockReason           the one blocking string, or null.
                                               The sentence PRINTS the storage
                                               half of it, so the footer never
                                               repeats those cases — it only
                                               renders the custom range refusal
                                               and holds Continue.
                    flow.storageProbe          the last probe, or null
                    flow.features.semanticSearch  the one download the user can
                                               drop to clear a downloads shortfall
                    flow.controller.draftFrameRate / draftSaveDirectory /
                    draftRetentionPolicy / appPrivacyExclusion
                    flow.controller.customResolutionErrors / customBitrateErrors
      onContinue  () => void — advance to Your settings. THE SHELL REFUSES IT while
                  `flow.blockReason` is non-null, so the button renders as held
                  (with the reason) rather than silently doing nothing.
      onBack      () => void — return to Permissions.
    emits
      flow.storageProbe = { exists, writable, freeBytes }  — THIS SCREEN IS THE
      ONLY WRITER, via the `probe_storage_path` command. `freeBytes: null` when
      the volume cannot be read; a null probe never blocks (an inability to
      measure is not a shortfall — ADR 0040).
    must not
      Add a third gate. Restate a failure the sentence already prints.
    gates
      1. storage path exists and is writable
      2. the volume has room for reserve + downloads + a day of capture
      plus custom resolution (16-8192 px) / bitrate (1-40 Mbps) range validation.
      The BLOCK DECISION is `flow.blockReason` (from $lib/onboarding/gates).
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import CaptureSentence from "$lib/onboarding/CaptureSentence.svelte";
  import ExcludedApps from "$lib/onboarding/ExcludedApps.svelte";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";

  let {
    flow,
    onContinue,
    onBack,
  }: { flow: OnboardingFlow; onContinue: () => void; onBack: () => void } = $props();

  const c = $derived(flow.controller);

  // ── The probe ────────────────────────────────────────────────────────────
  // Probe the chosen directory whenever it changes, AND on demand: re-picking the
  // same folder after fixing its permissions is an equal `$state` write, so the
  // effect alone would never re-run and the user could not recover without
  // restarting onboarding. `seq` discards a late reply from a directory the user
  // has already moved on from.
  let seq = 0;
  /** The path the command actually probed — the resolved default when the draft
   *  is blank, so the sentence never shows an empty folder. */
  let probedPath = $state("");
  /** "not yet probed" and "the probe itself failed" are DIFFERENT: the second
   *  used to render as "checking…" forever. `flow.storageProbe` still goes null
   *  on failure so the gate stays open (ADR 0040) — the sentence reports it. */
  let probeState = $state<"checking" | "done" | "failed">("checking");

  function runProbe(path: string) {
    const ticket = ++seq;
    probeState = "checking";
    void invoke<{
      path: string;
      exists: boolean;
      writable: boolean;
      freeBytes: number | null;
    }>("probe_storage_path", { path })
      .then((probe) => {
        if (ticket !== seq) return;
        probedPath = probe.path;
        flow.storageProbe = {
          exists: probe.exists,
          writable: probe.writable,
          freeBytes: probe.freeBytes,
        };
        probeState = "done";
      })
      .catch(() => {
        if (ticket !== seq) return;
        // Unmeasurable is not a shortfall (ADR 0040): leave the gate open, but
        // say so instead of pretending the check is still running.
        probedPath = path;
        flow.storageProbe = null;
        probeState = "failed";
      });
  }

  $effect(() => {
    runProbe(c.draftSaveDirectory);
  });

  // ── Seeding the privacy list ─────────────────────────────────────────────
  // On a first run the recommended exclusions are resolved DATA that `finish()`
  // applied at commit, so `draftExcludedApps` was empty here and "Never
  // recorded" rendered as nothing excluded — and a recommendation carries no
  // source id, so it could not be struck even if it were drawn. Seed on arrival
  // instead. The flow owns the one-shot (`finish()` is now the backstop);
  // reading the pending list keeps this reactive, so it fires as soon as the
  // recommendations land rather than racing them.
  $effect(() => {
    void c.appPrivacyExclusion.pendingRecommendedApps.length;
    void flow.seedRecommendedExcludedApps();
  });

  // ── The range refusal ────────────────────────────────────────────────────
  // The only blocking case the sentence does not own: a custom resolution or
  // bitrate typed on *Change settings* that serializes as `null`.
  const rangeField = $derived.by(() => {
    if (c.customResolutionErrors.length > 0) {
      return c.draftCustomWidth === null
        ? { value: `Width  ${c.customWidthRaw || "—"} px`, range: "16–8192 px" }
        : { value: `Height  ${c.customHeightRaw || "—"} px`, range: "16–8192 px" };
    }
    if (c.customBitrateErrors.length > 0) {
      return { value: `Bitrate  ${c.draftCustomMbpsRaw || "—"} Mbps`, range: "1–40 Mbps" };
    }
    return null;
  });
</script>

<!-- Centred in the stage: the shell's `.ob-foot` owns the bottom, so without this
     the heading and both surfaces sat at the top with ~200px of hole under them. -->
<div class="scene">
  <h1 class="ob-disp sm">Four things to settle.</h1>

  <div class="blocks">
    <CaptureSentence
      frameRate={c.draftFrameRate}
      onFrameRateChange={(fps) => (c.draftFrameRate = fps)}
      retention={c.draftRetentionPolicy}
      onRetentionChange={(policy) => (c.draftRetentionPolicy = policy)}
      saveDirectory={c.draftSaveDirectory}
      onSaveDirectoryChange={(path) => (c.draftSaveDirectory = path)}
      {probedPath}
      probe={flow.storageProbe}
      {probeState}
      onRecheck={() => runProbe(c.draftSaveDirectory)}
      requiredBytes={flow.downloadBytes}
      semanticSearchOn={flow.features.semanticSearch}
      onDisableSemanticSearch={() => flow.toggleFeature("semanticSearch")}
      onError={(message) => (c.errorMessage = message)}
    />

    <div class="privacy">
      <ExcludedApps privacy={c.appPrivacyExclusion} />
    </div>
  </div>
</div>

<div class="ob-foot">
  <!-- The storage gates print themselves inside the sentence, in the same order
       `captureStorageBlockReason` tests them. The footer only carries what the
       sentence does not own, and the held action row. -->
  {#if rangeField}
    <div class="field bad range">
      <span class="path">{rangeField.value}</span>
      <span class="mark">{rangeField.range}</span>
    </div>
  {/if}

  <hr class="ob-rule" />
  <div class="ob-acts">
    <button class="ob-btn ghost spacer" onclick={onBack}>← Back</button>
    {#if !flow.canContinue}
      <span class="ob-blocked">Continue held</span>
    {/if}
    <button class="ob-btn primary" onclick={onContinue} disabled={!flow.canContinue}>
      Your settings&nbsp; →
    </button>
  </div>
</div>

<style>
  /* Same idiom as ChangeSettingsScreen's `.scene` and SetupScreen's `.mid`:
     `safe center` centres the heading and both surfaces while they fit, and
     falls back to top-aligned the moment they do not — so a long "Never
     recorded" list scrolls from its first line instead of losing its head off
     the top of the stage. No `gap`: `.blocks` already carries its own margin. */
  .scene {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    justify-content: safe center;
    /* See `--ob-bleed`: the capture sentence's peek rule sits 14px into the
       gutter, and a pane that does not reserve it scrolls sideways instead. */
    padding-inline: var(--ob-bleed);
    margin-inline: calc(-1 * var(--ob-bleed));
  }
  .blocks {
    display: flex;
    flex-direction: column;
    gap: 26px;
    margin-top: 30px;
  }
  /* Full width, not the old 196px-label grid: `<ExcludedApps>` prints its own
     "Never recorded" label and its sentence needs the whole measure. */
  .privacy {
    border-top: 1px solid var(--app-border);
    padding-top: 18px;
  }

  /* The custom-value refusal, marked on the field with its range spelled out. */
  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    padding: 9px 12px;
    background: var(--app-surface-subtle);
    color: var(--app-text);
    font-size: var(--text-base);
  }
  .field.bad {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .field .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .field .mark {
    color: var(--app-danger);
    font-size: var(--text-sm);
    white-space: nowrap;
  }
  .field.range {
    width: fit-content;
    margin-bottom: 16px;
  }
</style>
