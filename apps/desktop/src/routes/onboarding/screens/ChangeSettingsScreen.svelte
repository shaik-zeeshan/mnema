<!--
  Screen 5 / 8 — Change settings (issue #195, slices 6 + 8-11).

  The flow's ONE deliberately dense screen: every detail demoted off the other
  seven lands here. Since the input components landed it is a THIN shell — four
  sections, one component each, and the section rail that reaches them:

    What Mnema will do   $lib/onboarding/FeatureSwitches   the eight switches
    Engines & models     $lib/onboarding/Providers         the engine + OCR line
                         $lib/onboarding/ModelPickers      family/variant/budget
    Storage              $lib/onboarding/CaptureSentence   rate + retention + place
    AI features          $lib/onboarding/AiSetup           providers, key, model

  Two rules this screen still carries:
   · `flow.toggleFeature(id)` is the ONLY way to flip a feature — it runs
     `applyToggle`, whose cascades run BOTH directions.
   · The single gate: AI features may not be left on with no credentials
     (enabled-but-unconfigured is a silent failure). Everything else is free.
     Deepgram is never selectable here — cloud transcription is Settings-only,
     behind its own consent gate (ADR 0047).

  One control per value: the retention picker and the capture-rate `Select` that
  used to live in Storage are GONE. `CaptureSentence` is the same control the
  Capture & Storage screen renders, so one user never meets one setting wearing
  two faces.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import AiSetup from "$lib/onboarding/AiSetup.svelte";
  import CaptureSentence from "$lib/onboarding/CaptureSentence.svelte";
  import FeatureSwitches from "$lib/onboarding/FeatureSwitches.svelte";
  import ModelPickers from "$lib/onboarding/ModelPickers.svelte";
  import Providers from "$lib/onboarding/Providers.svelte";
  import type { ProbeState } from "$lib/onboarding/capture-sentence";
  import { SPEAKRS_BYTES } from "$lib/onboarding/resolve-setup";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";

  let {
    flow,
    onContinue,
    onBack,
  }: { flow: OnboardingFlow; onContinue: () => void; onBack: () => void } = $props();

  const c = $derived(flow.controller);
  const f = $derived(flow.features);
  const ai = $derived(flow.controller.ai);

  // ── Section rail ──────────────────────────────────────────────────────────
  const SECTIONS = [
    { id: "features", label: "What Mnema will do" },
    { id: "engines", label: "Engines & models" },
    { id: "storage", label: "Storage" },
    { id: "ai", label: "AI features" },
  ] as const;
  let pane = $state<HTMLDivElement | null>(null);
  let active = $state<string>("features");

  function jumpTo(id: string): void {
    pane?.querySelector(`#cz-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
    active = id;
  }
  // Scroll-spy: the last section whose top has passed the pane's top edge. The
  // final section is shorter than the pane, so it can never reach that edge —
  // hitting the bottom selects it outright.
  function spy(): void {
    if (!pane) return;
    if (pane.scrollTop + pane.clientHeight >= pane.scrollHeight - 4) {
      active = SECTIONS[SECTIONS.length - 1].id;
      return;
    }
    const top = pane.scrollTop + 24;
    for (const section of SECTIONS) {
      const el = pane.querySelector<HTMLElement>(`#cz-${section.id}`);
      if (el && el.offsetTop <= top) active = section.id;
    }
  }

  // ── What the downloads will fetch ─────────────────────────────────────────
  // `flow.downloadBytes` is live in both the feature toggles and the model
  // pickers on this very screen. This is the DOWNLOAD work-list, not
  // `flow.requiredBytes` — the sentence adds the reserve and a day of capture
  // itself.
  const downloadBytes = $derived(flow.downloadBytes);

  // ── Storage probe ─────────────────────────────────────────────────────────
  // The sentence's folder token can move the directory from HERE, so this screen
  // measures what it prints rather than trusting the probe Capture & Storage
  // left behind. `seq` discards a late reply from a folder already moved on
  // from; a failed probe still leaves the gate open (ADR 0040) and says so.
  //
  // ponytail: a second copy of Capture & Storage's probe, not a shared owner —
  // hoisting it onto the flow buys one deduplicated effect and a third writer of
  // flow state. Hoist if a third screen ever needs it.
  let seq = 0;
  let probedPath = $state("");
  let probeState = $state<ProbeState>("checking");

  function runProbe(path: string): void {
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
        probedPath = path;
        flow.storageProbe = null;
        probeState = "failed";
      });
  }

  $effect(() => {
    runProbe(c.draftSaveDirectory);
  });

  // ── The one gate: AI on with nothing to run it ────────────────────────────
  const aiBlocked = $derived(f.aiFeatures && !ai.aiConfigReady);

  // Configuring a provider ENABLES the feature — otherwise the user verifies a
  // key and nothing uses it. Fires only on the false→true transition (and
  // `aiConfigReady` is now verification-gated), so turning the row off
  // afterwards with a working config is not undone. THE ONLY auto-enable site.
  let wasReady = false;
  $effect(() => {
    const ready = ai.aiConfigReady;
    if (ready && !wasReady && !flow.features.aiFeatures) flow.toggleFeature("aiFeatures");
    wasReady = ready;
  });
</script>

<div class="head">
  <h1 class="ob-disp sm">You asked. Here is all of it.</h1>
  <span class="ob-fine">Going back re-resolves your settings.</span>
</div>

<div class="cz">
  <nav class="idx" aria-label="Sections">
    <span class="ob-m">Sections</span>
    {#each SECTIONS as section (section.id)}
      <button
        class="idx-link"
        class:on={active === section.id}
        type="button"
        onclick={() => jumpTo(section.id)}
      >
        {section.label}
      </button>
    {/each}
  </nav>

  <div class="scrollpane" bind:this={pane} onscroll={spy}>
    <!-- ── The eight switches, as one chain ────────────────────────────── -->
    <section class="sec" id="cz-features">
      <FeatureSwitches
        features={f}
        onToggle={(id) => flow.toggleFeature(id)}
        onRestore={(snapshot) => (flow.features = snapshot)}
        onGrant={() => void c.requestPermission("microphone")}
        aiConfigured={ai.aiConfigReady}
        onConnectAi={() => jumpTo("ai")}
        aiNote={ai.aiConfigMissing}
        models={flow.models}
        installed={flow.modelFacts}
        captureIntervalSeconds={flow.captureIntervalSeconds}
      />
    </section>

    <!-- ── Engines & models ────────────────────────────────────────────── -->
    <section class="sec" id="cz-engines">
      <!-- Providers owns the ENGINE choice (with its reason and its price);
           ModelPickers owns the family's builds and the budget, so its own
           family group is left empty here rather than drawn twice. -->
      <Providers
        providers={c.transcriptionModelStatus?.providers ?? []}
        value={c.draftTranscriptionProvider}
        onValueChange={(v) => c.chooseTranscriptionProvider(v)}
        ocrProviders={c.ocrModelStatus?.providers ?? []}
        ocrProvider={c.draftOcrProvider}
      />
      <hr class="ob-rule sep" />
      <ModelPickers
        transcriptionFamilies={[]}
        transcriptionFamily={c.draftTranscriptionProvider}
        transcriptionModels={c.selectedTranscriptionModels}
        transcriptionModelId={c.draftTranscriptionModelId}
        transcriptionEnabled={f.transcription}
        onTranscriptionFamilyChange={(v) => c.chooseTranscriptionProvider(v)}
        onTranscriptionModelChange={(v) => c.chooseTranscriptionModel(v)}
        semanticStatus={c.semanticSearchModelStatus?.models ?? []}
        semanticCatalog={c.semanticSearchSupportedModels}
        semanticModelId={c.draftSemanticSearchModelId}
        semanticEnabled={f.semanticSearch}
        onSemanticModelChange={(v) => c.chooseSemanticSearchModel(v)}
        onSemanticEnabledChange={(on) => {
          if (on !== f.semanticSearch) flow.toggleFeature("semanticSearch");
        }}
        speakerEnabled={f.speakerSeparation}
        speakerInstalled={c.selectedSpeakerModel?.available ?? false}
        speakerBytes={c.selectedSpeakerModel?.download?.byteSize ?? SPEAKRS_BYTES}
        freeBytes={flow.storageProbe?.freeBytes ?? null}
        captureIntervalSeconds={flow.captureIntervalSeconds}
      />
    </section>

    <!-- ── Storage: the same one line the Capture & Storage screen shows ── -->
    <section class="sec" id="cz-storage">
      <span class="ob-m">Storage</span>
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
        requiredBytes={downloadBytes}
        semanticSearchOn={f.semanticSearch}
        onDisableSemanticSearch={() => flow.toggleFeature("semanticSearch")}
        onError={(message) => (c.errorMessage = message)}
      />
    </section>

    <!-- ── AI features ─────────────────────────────────────────────────── -->
    <section class="sec" id="cz-ai">
      <!-- AiSetup only prints this inside its "set it up now" mode, and a
           default model cleared on re-entry needs its reason visible whichever
           mode the user is in. -->
      {#if ai.aiRestoredModelNote}
        <p class="ob-fine err">{ai.aiRestoredModelNote}</p>
      {/if}
      <AiSetup {ai} aiEnabled={f.aiFeatures} onToggleAi={() => flow.toggleFeature("aiFeatures")} />
    </section>
  </div>
</div>

<div class="ob-foot">
  <hr class="ob-rule" />
  <div class="ob-acts">
    {#if aiBlocked}
      <!-- The only rule this screen enforces. Enabled-but-unconfigured is a
           silent failure, so Back is held until it is resolved either way. -->
      <span class="ob-blocked spacer">
        Back held · AI features are on, with nothing to run them
      </span>
      <button class="ob-btn" type="button" onclick={() => jumpTo("ai")}>Add a key</button>
      <button class="ob-btn" type="button" onclick={() => flow.toggleFeature("aiFeatures")}>
        Turn AI features off
      </button>
    {:else}
      <span class="ob-fine spacer">
        Turn the last audio source off and transcription and who's speaking switch off with it.
      </span>
      <button class="ob-btn ghost" onclick={onBack}>← Back</button>
      <button class="ob-btn primary" onclick={onContinue}>Back to your settings&nbsp; →</button>
    {/if}
  </div>
</div>

<style>
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 24px;
    margin-bottom: 18px;
  }

  /* Rail + scrolling detail pane — the only two-column screen in the flow. */
  .cz {
    display: grid;
    grid-template-columns: 150px 1fr;
    gap: 34px;
    flex: 1;
    min-height: 0;
  }
  .idx {
    display: flex;
    flex-direction: column;
    gap: 11px;
    border-right: 1px solid var(--app-border);
    padding-right: 20px;
  }
  .idx-link {
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    color: var(--app-text-subtle);
    background: transparent;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .idx-link.on {
    color: var(--app-text-strong);
  }
  .idx-link:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  .scrollpane {
    position: relative;
    min-height: 0;
    overflow-y: auto;
    padding-right: 4px;
  }
  .sec {
    padding: 0 18px 20px 0;
  }
  .sec + .sec {
    border-top: 1px solid var(--app-border);
    padding-top: 18px;
  }
  /* `.ob-rule` ships margin:0; the engine cards and the pickers are two blocks,
     not one. */
  .sep {
    margin: 22px 0;
  }

  .err {
    color: var(--app-danger);
    margin: 0 0 10px;
  }
  .spacer {
    margin-right: auto;
  }
</style>
