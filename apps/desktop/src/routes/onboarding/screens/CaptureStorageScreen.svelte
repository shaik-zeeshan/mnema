<!--
  Screen 3 / 8 — Capture & Storage. THE ONLY SCREEN IN THE FLOW WITH HARD GATES.

  Four spec rows and a verdict panel: `<CaptureSentence>` (capture rate +
  retention + folder, and every failure/repair those three can produce) with
  `<ExcludedApps>` slotted in as the fourth row ("Never recorded"). Both live in
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
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconArrow from "~icons/lucide/arrow-right";

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

<div class="ob-main">
  <div class="ob-col">
    <div class="ob-head">
      <span class="mx-kicker">Capture &amp; Storage</span>
      <h1 class="mx-display">Four things to settle.</h1>
    </div>

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
      videoPixels={flow.videoPixels}
      semanticSearchOn={flow.features.semanticSearch}
      onDisableSemanticSearch={() => flow.toggleFeature("semanticSearch")}
      onError={(message) => (c.errorMessage = message)}
    >
      {#snippet rows()}
        <ExcludedApps privacy={c.appPrivacyExclusion} />
      {/snippet}
    </CaptureSentence>

    <!-- The storage gates print themselves in the verdict panel, in the same
         order `captureStorageBlockReason` tests them. This is the one block the
         panel does not own: a custom value typed on *Change settings*. -->
    {#if rangeField}
      <span class="mx-inline" data-tone="danger">
        <IconAlert width="13" height="13" /><b>{rangeField.value}</b> — allowed {rangeField.range}
      </span>
    {/if}

    <p class="ob-fine">
      Estimate measured on a real Mac: 270 MB a day at one snapshot every 3 s, pause-on-inactivity on.
    </p>
  </div>
</div>

<footer class="ob-foot">
  <p class="ob-foot__note"></p>
  <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
  {#if !flow.canContinue}<span class="mx-chip mx-chip--warn">continue held</span>{/if}
  <button class="mx-btn mx-btn--primary" onclick={onContinue} disabled={!flow.canContinue}>
    Your settings<IconArrow class="ob-btn-arrow" width="14" height="14" />
  </button>
</footer>
