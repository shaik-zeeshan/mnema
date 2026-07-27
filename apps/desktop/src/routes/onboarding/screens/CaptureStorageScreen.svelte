<!--
  Screen 3 / 8 — Capture & Storage. THE ONLY SCREEN IN THE FLOW WITH HARD GATES.

  CONTRACT
    props
      flow        OnboardingFlow. Reads:
                    flow.requiredBytes         total bytes the download work-list will fetch
                    flow.blockReason           the one blocking string, or null (already
                                               computed by the shell from the probe below —
                                               render it, do not re-derive it)
                    flow.storageProbe          the last probe, or null
                    flow.controller.draftFrameRate                 fps; the UI speaks in
                                                                   "one snapshot every X"
                                                                   ($lib/components/capture-rate)
                    flow.controller.draftSaveDirectory
                    flow.controller.draftRetentionPolicy           default "never"
                    flow.controller.draftExcludedApps              ExcludedAppEntry[]
                    flow.controller.customResolutionErrors / customBitrateErrors
                    flow.controller.appPrivacyExclusion            excluded-apps editor
      onContinue  () => void — advance to Your settings. THE SHELL REFUSES IT while
                  `flow.blockReason` is non-null, so the button renders as held
                  (with the reason) rather than silently doing nothing.
      onBack      () => void — return to Permissions.
    emits
      flow.storageProbe = { exists, writable, freeBytes }  — THIS SCREEN IS THE
      ONLY WRITER, via the `probe_storage_path` command. `freeBytes: null` when
      the volume cannot be read; a null probe never blocks (an inability to
      measure is not a shortfall — ADR 0040).
    owns
      The capture-rate slider, the storage location + folder picker
      (@tauri-apps/plugin-dialog, never window.confirm), the four Retention
      options with ONE consequence line under the selected one, the excluded-apps
      summary + Edit affordance, and the real error copy for both hard gates.
    must not
      Add a third gate. Show a retention chart. Annotate the unselected
      retention options.
    gates
      1. storage path exists and is writable
      2. the volume has room for `flow.requiredBytes`
      plus custom resolution (16-8192 px) / bitrate (1-40 Mbps) range validation.
      The BLOCK DECISION is `flow.blockReason` (from $lib/onboarding/gates); only
      the *presentation* is chosen here, in the same order the predicate tests.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    intervalSToFps,
    nearestLadderIndex,
  } from "$lib/components/capture-rate";
  import { retentionToDays } from "$lib/components/retention";
  import {
    estimateDailyStorageMb,
    estimateWindowStorageMb,
  } from "$lib/onboarding/disk-estimate";
  import { NOMIC_BYTES } from "$lib/onboarding/resolve-setup";
  import { describeError } from "$lib/settings/state/format";
  import type { RetentionPolicy } from "$lib/types";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";

  let {
    flow,
    onContinue,
    onBack,
  }: { flow: OnboardingFlow; onContinue: () => void; onBack: () => void } = $props();

  const c = $derived(flow.controller);

  // ── 1. How often ─────────────────────────────────────────────────────────
  // The app's existing log-spaced ladder, driven by a native range input over
  // its indices. The wire format stays fps; the UI never says "fps".
  const LAST = CAPTURE_INTERVAL_LADDER_S.length - 1;
  const ladderIndex = $derived(nearestLadderIndex(flow.controller.draftFrameRate));
  const intervalS = $derived(CAPTURE_INTERVAL_LADDER_S[ladderIndex]!);
  const dailyMb = $derived(estimateDailyStorageMb(intervalS));

  function setLadderIndex(raw: string) {
    const i = Math.min(Math.max(Number(raw) | 0, 0), LAST);
    c.draftFrameRate = intervalSToFps(CAPTURE_INTERVAL_LADDER_S[i]!);
  }

  // ── 2. Where it lives ────────────────────────────────────────────────────
  // Probe the chosen directory whenever it changes. `seq` discards a late reply
  // from a directory the user has already moved on from.
  let seq = 0;
  let browsing = $state(false);
  /** The path the command actually probed — the resolved default when the draft
   *  is blank, so the row never shows an empty box. */
  let probedPath = $state("");

  $effect(() => {
    const path = c.draftSaveDirectory;
    const ticket = ++seq;
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
      })
      .catch(() => {
        if (ticket !== seq) return;
        // Unmeasurable is not a shortfall (ADR 0040): leave the gate open.
        probedPath = path;
        flow.storageProbe = null;
      });
  });

  async function browse() {
    if (browsing) return;
    browsing = true;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose where Mnema stores captures",
        defaultPath: c.draftSaveDirectory || undefined,
      });
      if (typeof picked === "string" && picked.trim().length > 0) {
        c.draftSaveDirectory = picked;
      }
    } catch (err) {
      c.errorMessage = `Couldn't open the folder picker: ${describeError(err)}`;
    } finally {
      browsing = false;
    }
  }

  const home = $derived(probedPath.match(/^(\/Users\/[^/]+)/)?.[1] ?? null);
  const shownPath = $derived(home ? `~${probedPath.slice(home.length)}` : probedPath);
  const freeLabel = $derived.by(() => {
    const probe = flow.storageProbe;
    if (!probe) return "checking…";
    if (!probe.exists) return "doesn't exist";
    const room = probe.freeBytes === null ? "free space unknown" : `${si(probe.freeBytes)} free`;
    return `${room} · ${probe.writable ? "writable" : "read-only"}`;
  });

  // ── 3. How long to keep it ───────────────────────────────────────────────
  // "Keep everything" FIRST and default: retention is the only setting whose
  // wrong guess destroys data with no undo, so the app never defaults to
  // deleting. Exactly one consequence line, under the SELECTED option only.
  const RETENTION: { value: RetentionPolicy; label: string }[] = [
    { value: "never", label: "Keep everything" },
    { value: "days_30", label: "30 days" },
    { value: "days_14", label: "14 days" },
    { value: "days_7", label: "7 days" },
  ];
  const retentionNote = $derived.by(() => {
    const days = retentionToDays(c.draftRetentionPolicy);
    if (days === null) {
      return `About ${mb(dailyMb)} a day with no limit — roughly ${mb(dailyMb * 30)} a month.`;
    }
    return `About ${mb(estimateWindowStorageMb(intervalS, days))} held at any time — older than ${days} days is deleted.`;
  });

  // ── 4. Never recorded ────────────────────────────────────────────────────
  // A summary, not an inline picker — the editor is behind the Edit affordance.
  // On a first run the recommended list is resolved DATA (applied at commit), so
  // it counts here even though `draftExcludedApps` does not hold it yet.
  let editingExclusions = $state(false);
  const excludedNames = $derived.by(() => {
    const seen = new Set<string>();
    const names: string[] = [];
    const push = (bundleId: string, displayName: string) => {
      if (seen.has(bundleId)) return;
      seen.add(bundleId);
      names.push(displayName || bundleId);
    };
    for (const app of c.draftExcludedApps) if (app.enabled) push(app.bundleId, app.displayName);
    if (flow.resolved?.applyRecommendedExcludedApps) {
      for (const app of c.appPrivacyExclusion.pendingRecommendedApps) {
        push(app.bundleId, app.displayName);
      }
    }
    return names;
  });
  const excludedSummary = $derived(
    excludedNames.length === 0
      ? "Nothing excluded yet"
      : `${excludedNames.slice(0, 3).join(", ")}${excludedNames.length > 3 ? "…" : ""}`,
  );

  // ── The gates ────────────────────────────────────────────────────────────
  // `flow.blockReason` decides WHETHER we are held; this only picks which panel
  // says so, in the order `captureStorageBlockReason` tests its cases.
  type GateKind = "missing" | "unwritable" | "room" | "range";
  const gate = $derived.by<GateKind | null>(() => {
    if (flow.blockReason === null) return null;
    const probe = flow.storageProbe;
    if (probe) {
      if (!probe.exists) return "missing";
      if (!probe.writable) return "unwritable";
      if (probe.freeBytes !== null && probe.freeBytes < flow.requiredBytes) return "room";
    }
    return "range";
  });

  /** The offending custom value, refused on the field with its range spelled out. */
  const rangeField = $derived.by(() => {
    if (c.customResolutionErrors.length > 0) {
      return c.draftCustomWidth === null
        ? { value: `Width  ${c.customWidthRaw || "—"} px`, range: "16–8192 px" }
        : { value: `Height  ${c.customHeightRaw || "—"} px`, range: "16–8192 px" };
    }
    return { value: `Bitrate  ${c.draftCustomMbpsRaw || "—"} Mbps`, range: "1–40 Mbps" };
  });

  // Byte figures are DYNAMIC and quoted decimally: the download total contains
  // nomic's `approx_download_bytes`, so it is presented as "about N", never as
  // an exact number.
  function si(bytes: number): string {
    if (bytes >= 1e9) {
      const gb = bytes / 1e9;
      return `${gb >= 100 ? Math.round(gb) : gb.toFixed(1)} GB`;
    }
    if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
    if (bytes >= 1e3) return `${Math.round(bytes / 1e3)} KB`;
    return `${Math.round(bytes)} B`;
  }
  function mb(megabytes: number): string {
    return si(megabytes * 1e6);
  }
</script>

<h1 class="ob-disp sm">Four things to settle.</h1>

<div class="blocks">
  <div class="blk">
    <span class="ob-m">How often</span>
    <div class="val">
      <div class="lad-wrap">
        <span class="lad-track" aria-hidden="true"></span>
        <span class="lad-fill" style="width:{(ladderIndex / LAST) * 100}%" aria-hidden="true"></span>
        {#each CAPTURE_INTERVAL_LADDER_S as stop, i (stop)}
          <span class="lad-stop" style="left:{(i / LAST) * 100}%" aria-hidden="true"></span>
        {/each}
        <input
          class="lad"
          type="range"
          min="0"
          max={LAST}
          step="1"
          value={ladderIndex}
          oninput={(e) => setLadderIndex(e.currentTarget.value)}
          aria-label="Capture rate"
          aria-valuetext={captureIntervalPhrase(intervalS)}
        />
      </div>
      <span class="ob-body readout">
        {captureIntervalPhrase(intervalS)} &nbsp;·&nbsp;
        <span class="ob-num">~{mb(dailyMb)}</span> a day
      </span>
    </div>
  </div>

  <div class="blk">
    <span class="ob-m">Where it lives</span>
    <div class="val">
      <div class="field" class:bad={gate === "missing" || gate === "unwritable"}>
        <span class="path">{shownPath || "…"}</span>
        <button class="ob-btn sm" onclick={browse} disabled={browsing}>
          {browsing ? "Choosing…" : "Change"}
        </button>
      </div>
      <span class="ob-fine" class:bad-note={gate === "missing" || gate === "unwritable"}>
        {freeLabel}
      </span>
    </div>
  </div>

  <div>
    <div class="blk">
      <span class="ob-m">How long to keep it</span>
      <div class="val">
        <div class="seg" role="group" aria-label="How long to keep it">
          {#each RETENTION as option (option.value)}
            <button
              type="button"
              aria-pressed={c.draftRetentionPolicy === option.value}
              class:on={c.draftRetentionPolicy === option.value}
              onclick={() => (c.draftRetentionPolicy = option.value)}
            >
              {option.label}
            </button>
          {/each}
        </div>
      </div>
    </div>
    <p class="blk-note">{retentionNote}</p>
  </div>

  <div class="blk">
    <span class="ob-m">Never recorded</span>
    <div class="val">
      <span class="ob-body summary">
        <span class="ob-strong">{excludedNames.length} apps</span> — {excludedSummary}
      </span>
      <button
        class="ob-btn sm"
        aria-expanded={editingExclusions}
        onclick={() => (editingExclusions = !editingExclusions)}
      >
        {editingExclusions ? "Done" : "Edit  ▸"}
      </button>
    </div>
  </div>

  {#if editingExclusions}
    <div class="editor">
      <AppPrivacyExclusion
        controller={c.appPrivacyExclusion}
        comboboxListId="capture-storage-privacy-app-list"
      />
    </div>
  {/if}
</div>

<div class="ob-foot">
  <!-- One line, then the held action row. The offending path itself is already
       marked on the "Where it lives" row above, so the gate does not repeat it —
       that is what keeps the blocked screen inside the same 1040x680 frame. -->
  {#if gate === "missing" || gate === "unwritable"}
    <p class="gate">
      <span class="hd">
        {gate === "unwritable" ? "That folder is not writable" : "That folder doesn't exist"}
      </span>
      Named before recording begins, not after a day of lost capture.
    </p>
  {:else if gate === "room"}
    <p class="gate">
      <span class="hd">Not enough room for the downloads</span>
      <span class="ob-strong ob-num">
        {si(flow.storageProbe?.freeBytes ?? 0)} free · about {si(flow.requiredBytes)} needed.
      </span>
      Caught before the first byte is fetched.
    </p>
  {:else if gate === "range"}
    <div class="field bad range">
      <span class="path">{rangeField.value}</span>
      <span class="mark">{rangeField.range}</span>
    </div>
  {:else}
    <p class="ob-fine footnote">
      Measured: <span class="ob-num">270 MB</span> a day at one snapshot every 3 seconds, with
      pause-on-inactivity on. Every figure above is that, scaled.
    </p>
  {/if}

  <hr class="ob-rule" />
  <div class="ob-acts">
    <button class="ob-btn ghost spacer" onclick={onBack}>← Back</button>
    {#if gate !== null}
      <span class="ob-blocked">Continue held</span>
    {/if}
    {#if gate === "missing" || gate === "unwritable"}
      <button class="ob-btn sm" onclick={browse} disabled={browsing}>Choose another folder</button>
    {:else if gate === "room"}
      <button class="ob-btn sm" onclick={browse} disabled={browsing}>Change folder</button>
      <span class="ob-fine">or turn Semantic&nbsp;Search off — {si(NOMIC_BYTES)}.</span>
    {/if}
    <button class="ob-btn primary" onclick={onContinue} disabled={!flow.canContinue}>
      Your settings&nbsp; →
    </button>
  </div>
</div>

<style>
  /* Four rows, one line each, ~32px apart. Ported from the mockup's `.blocks` /
     `.blk` (frame 03) with every colour as an `--app-*` token. */
  .blocks {
    display: flex;
    flex-direction: column;
    gap: 32px;
    margin-top: 36px;
  }
  .blk {
    display: grid;
    grid-template-columns: 196px 1fr;
    gap: 32px;
    align-items: center;
    padding: 9px 0;
    border-top: 1px solid var(--app-border);
  }
  .blocks > .blk:first-child {
    border-top: 0;
    padding-top: 2px;
  }
  .val {
    display: flex;
    align-items: center;
    gap: 22px;
  }
  .readout {
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .summary {
    color: var(--app-text);
  }
  /* The one consequence line a row is allowed, hung under the value column. */
  .blk-note {
    margin: 12px 0 0 228px;
    font-size: var(--text-md);
    color: var(--app-text);
  }

  /* ---- the capture-rate ladder: the app's log-spaced stops, on a native
         range input so keyboard + pointer behaviour comes free ---- */
  .lad-wrap {
    position: relative;
    width: 396px;
    height: 30px;
    flex: none;
  }
  .lad-track,
  .lad-fill {
    position: absolute;
    top: 14px;
    height: 2px;
    border-radius: 2px;
  }
  .lad-track {
    left: 0;
    right: 0;
    background: var(--app-border-strong);
  }
  .lad-fill {
    left: 0;
    background: var(--app-text-muted);
  }
  .lad-stop {
    position: absolute;
    top: 9px;
    width: 1px;
    height: 12px;
    margin-left: -0.5px;
    background: var(--app-border-hover);
  }
  .lad {
    position: absolute;
    inset: 0;
    width: 100%;
    margin: 0;
    background: transparent;
    -webkit-appearance: none;
    appearance: none;
  }
  .lad::-webkit-slider-runnable-track {
    height: 30px;
    background: transparent;
  }
  .lad::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 20px;
    border-radius: 3px;
    border: 0;
    background: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .lad:focus-visible::-webkit-slider-thumb {
    outline: 2px solid var(--app-accent);
    outline-offset: 2px;
  }

  /* ---- shared primitives (mockup `.field`, `.seg`) ---- */
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
    min-width: 300px;
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

  .seg {
    display: inline-flex;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    overflow: hidden;
  }
  .seg button {
    font: inherit;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    background: transparent;
    border: 0;
    border-right: 1px solid var(--app-border);
    padding: 9px 16px;
    cursor: pointer;
    white-space: nowrap;
  }
  .seg button:last-child {
    border-right: 0;
  }
  .seg button:hover {
    color: var(--app-text);
  }
  .seg button.on {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  .seg button:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* The excluded-apps editor, revealed by Edit — the default state is the
     one-line summary above, never this. */
  .editor {
    border-top: 1px solid var(--app-border);
    padding-top: 18px;
  }

  /* ---- the two hard gates + the range refusal ---- */
  .gate {
    margin: 0 0 16px;
    font-size: var(--text-md);
    line-height: 1.7;
    color: var(--app-text-muted);
  }
  .gate .hd {
    font-size: var(--text-lg);
    color: var(--app-text-strong);
    margin-right: 10px;
  }
  .field.range {
    min-width: 0;
    width: fit-content;
    margin-bottom: 16px;
  }
  .bad-note {
    color: var(--app-danger);
  }
  .footnote {
    max-width: none;
    margin-bottom: 16px;
  }
</style>
