<!--
  Screen 5 / 8 — Change settings (issue #195, slices 6 + 8-11, revision 2 slice 4).

  The flow's ONE deliberately dense screen: every detail demoted off the other
  seven lands here. Revision 2 stops it being a scroll: the four sections are
  SCREENS, one at a time behind an `mx-seg--md` tab strip, which replaced
  the 150px section rail (which held four links and then a column-height hole).

    What Mnema will do   $lib/onboarding/FeatureSwitches   the eight switches
    Engines              $lib/onboarding/Providers         the engine choice
    Models               $lib/onboarding/ModelPickers      family/variant/budget
    AI features          $lib/onboarding/AiSetup           providers, key, model

  The sections are internal state of this ONE step — `onboarding-flow.svelte.ts`
  are untouched, this is step 5 of 8 (its own short "round trip" tick), and the
  round trip still returns to Your settings.

  Two rules this screen still carries:
   · `flow.toggleFeature(id)` is the ONLY way to flip a feature — it runs
     `applyToggle`, whose cascades run BOTH directions.
   · AI features left on with no credentials is a soft warning, never a hold:
     it prints in place beside the row it is about, and after onboarding the
     engine pill, Insights and Settings → Providers all say "not set up".
     Deepgram is never selectable here — cloud transcription is Settings-only,
     behind its own consent gate (ADR 0047).

  One control per value: Storage is GONE as a section. It repeated the Capture &
  Storage screen's sentence verbatim, so rate, retention and folder are settled
  there — one Back away — and this screen no longer runs a second storage probe.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import AiSetup from "$lib/onboarding/AiSetup.svelte";
  import FeatureSwitches from "$lib/onboarding/FeatureSwitches.svelte";
  import ModelPickers from "$lib/onboarding/ModelPickers.svelte";
  import Providers from "$lib/onboarding/Providers.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconArrow from "~icons/lucide/arrow-right";
  import { buildEngines } from "$lib/onboarding/transcription-engines";
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

  // ── Sections ──────────────────────────────────────────────────────────────
  // `note` is the footer's left-hand state line for that section — the thing
  // worth knowing while you are on it.
  const SECTIONS = [
    {
      id: "features",
      label: "What Mnema will do",
      note: "Turn the last audio source off and transcription and who's speaking switch off with it.",
    },
    {
      id: "engines",
      label: "Engines",
      note: "Cloud transcription is not offered here — it lives in Settings behind its own consent gate.",
    },
    {
      id: "models",
      label: "Models",
      note: "Sizes come from the model manifests — nothing here is a pasted constant.",
    },
    {
      id: "ai",
      label: "AI features",
      note: "Nothing here is sent anywhere until you pick a provider.",
    },
  ] as const;

  let active = $state<(typeof SECTIONS)[number]["id"]>("features");
  const at = $derived(SECTIONS.findIndex((s) => s.id === active));

  // The engine is chosen on Engines and named here as context, so Models never
  // draws the family row a second time (`transcriptionFamilies={[]}` below).
  const engineName = $derived(
    buildEngines(c.transcriptionModelStatus?.providers ?? []).find(
      (e) => e.id === c.draftTranscriptionProvider,
    )?.name ?? c.draftTranscriptionProvider,
  );

  // ── AI on with nothing to run it: a warning in place, never a hold ────────
  const aiBlocked = $derived(f.aiFeatures && !ai.aiConfigReady);

  // Configuring a provider ENABLES the feature — otherwise the user verifies a
  // key and nothing uses it. Fires only on the false→true transition (and
  // `aiConfigReady` is now verification-gated), so turning the row off
  // afterwards with a working config is not undone. THE ONLY auto-enable site.
  //
  // SEEDED from the live readiness, never from `false`: this screen remounts on
  // every round trip through *Your settings*, while the AI store lives on the
  // controller and outlives it. A latch that started false would read an
  // already-ready engine as a fresh edge and silently switch AI features back on
  // for a user who had just turned the row off.
  let wasReady = untrack(() => flow.controller.ai.aiConfigReady);
  $effect(() => {
    const ready = ai.aiConfigReady;
    if (ready && !wasReady && !flow.features.aiFeatures) flow.toggleFeature("aiFeatures");
    wasReady = ready;
  });
</script>

<div class="ob-main change">
  <div class="ob-col">
    <div class="tabs">
      <Segmented
        md
        options={SECTIONS.map((s) => ({ value: s.id, label: s.label }))}
        value={active}
        ariaLabel="Sections"
        onValueChange={(v) => (active = v as typeof active)}
      />
      <span class="num">{at + 1} / {SECTIONS.length}</span>
    </div>

    {#if active === "features"}
      <!-- ── The eight switches, as one chain ──────────────────────────────── -->
      <FeatureSwitches
        features={f}
        onToggle={(id) => flow.toggleFeature(id)}
        onRestore={(snapshot) => (flow.features = snapshot)}
        onGrant={() => void c.requestPermission("microphone")}
        aiConfigured={ai.aiConfigReady}
        onConnectAi={() => (active = "ai")}
        aiNote={ai.aiConfigMissing}
        models={flow.models}
        installed={flow.modelFacts}
        captureIntervalSeconds={flow.captureIntervalSeconds}
        videoPixels={flow.videoPixels}
      />
    {:else if active === "engines"}
      <!-- ── Which engine turns speech into text, and why one is picked ────── -->
      <Providers
        providers={c.transcriptionModelStatus?.providers ?? []}
        value={c.draftTranscriptionProvider}
        onValueChange={(v) => c.chooseTranscriptionProvider(v)}
      />
    {:else if active === "models"}
      <!-- ── Which build of that engine, and what it costs against the disk ── -->
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
        videoPixels={flow.videoPixels}
      >
        {#snippet transcriptionNote()}
          Engine: <b>{engineName}</b> — chosen on
          <button class="link" type="button" onclick={() => (active = "engines")}>Engines</button>. This screen
          picks its build.
        {/snippet}
      </ModelPickers>
    {:else}
      <!-- ── AI features ──────────────────────────────────────────────────── -->
      <AiSetup {ai} aiEnabled={f.aiFeatures} onToggleAi={() => flow.toggleFeature("aiFeatures")}>
        {#snippet notices()}
          {#if aiBlocked}
            <!-- The warning, beside the row it is about. Never a hold. -->
            <div class="mx-notice" data-tone="warn">
              <span class="mx-notice__icon"><IconAlert width="15" height="15" /></span>
              <p class="mx-notice__text">
                The <b>AI features</b> row is on, with nothing to run it — connect a provider here, or switch
                the row off.
              </p>
            </div>
          {/if}
          <!-- AiSetup only prints this inside "set it up now"; a default model
               cleared on re-entry needs its reason visible in every mode. -->
          {#if ai.aiRestoredModelNote}
            <span class="mx-inline" data-tone="danger">
              <IconAlert width="13" height="13" />{ai.aiRestoredModelNote}
            </span>
          {/if}
        {/snippet}
      </AiSetup>
    {/if}
  </div>
</div>

<footer class="ob-foot">
  <p class="ob-foot__note">{SECTIONS[at].note}</p>
  {#if at === 0}
    <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back to your settings</button>
  {:else}
    <button class="mx-btn mx-btn--ghost" onclick={() => (active = SECTIONS[at - 1].id)}>
      {SECTIONS[at - 1].label}
    </button>
  {/if}
  {#if at === SECTIONS.length - 1}
    <button class="mx-btn mx-btn--primary" onclick={onContinue}>
      Back to your settings<IconArrow class="ob-btn-arrow" width="14" height="14" />
    </button>
  {:else}
    <button class="mx-btn mx-btn--primary" onclick={() => (active = SECTIONS[at + 1].id)}>
      {SECTIONS[at + 1].label}<IconArrow class="ob-btn-arrow" width="14" height="14" />
    </button>
  {/if}
</footer>

<style>
  /* Deliberately dense: top-aligned, wider column. */
  .change {
    align-content: start;
    padding-top: var(--s-5);
  }
  .change > .ob-col {
    max-width: 900px;
    gap: var(--s-5);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .link {
    font: inherit;
    color: var(--app-accent);
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: var(--app-accent-border);
    text-underline-offset: 3px;
  }
  .link:focus-visible {
    outline: none;
    border-radius: 2px;
    box-shadow: var(--app-ring);
  }
  .tabs .num {
    font: 400 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
</style>
