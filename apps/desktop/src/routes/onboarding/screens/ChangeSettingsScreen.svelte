<!--
  Screen 5 / 8 — Change settings (issue #195, slices 6 + 8-11, revision 2 slice 4).

  The flow's ONE deliberately dense screen: every detail demoted off the other
  seven lands here. Revision 2 stops it being a scroll: the four sections are
  SCREENS, one at a time behind a tab strip, and the tab strip is what replaced
  the 150px section rail (which held four links and then a column-height hole).

    What Mnema will do   $lib/onboarding/FeatureSwitches   the eight switches
    Engines              $lib/onboarding/Providers         the engine choice
    Models               $lib/onboarding/ModelPickers      family/variant/budget
    AI features          $lib/onboarding/AiSetup           providers, key, model

  The sections are internal state of this ONE step — `onboarding-flow.svelte.ts`
  and the phase bar are untouched, this is still step 4 of 8, and the round trip
  still returns to Your settings.

  Two rules this screen still carries:
   · `flow.toggleFeature(id)` is the ONLY way to flip a feature — it runs
     `applyToggle`, whose cascades run BOTH directions.
   · The single gate: AI features may not be left on with no credentials
     (enabled-but-unconfigured is a silent failure). It holds the two exits from
     this screen — never the tabs, so the fix is always one click away — and on
     the AI section it also prints in place, beside the row it is about.
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

  // ── The one gate: AI on with nothing to run it ────────────────────────────
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

<nav class="tabs" aria-label="Sections">
  {#each SECTIONS as section (section.id)}
    <button
      class="tab"
      class:on={active === section.id}
      type="button"
      aria-current={active === section.id ? "true" : undefined}
      onclick={() => (active = section.id)}
    >
      {section.label}
    </button>
  {/each}
  <span class="count">{at + 1} / {SECTIONS.length}</span>
</nav>

<!-- One section at a time. `.scene` is what scrolls at the 620px minimum, so
     the footer below it is never pushed off the stage. -->
<div class="scene">
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
    <p class="ob-fine ctx">
      Engine: <span class="ob-strong">{engineName}</span> — chosen on
      <button class="link" type="button" onclick={() => (active = "engines")}>Engines</button>. This
      screen picks its build.
    </p>
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
    />
  {:else}
    <!-- ── AI features ──────────────────────────────────────────────────── -->
    {#if aiBlocked}
      <!-- The gate, beside the row it is about. In the old scroll it only ever
           printed in the footer, several hundred pixels below the switch it
           names. -->
      <span class="ob-blocked gate">
        The <span class="ob-strong">AI features</span> row is on, with nothing to run it — connect a
        provider here, or switch the row off.
      </span>
    {/if}
    <!-- AiSetup only prints this inside its "set it up now" mode, and a
         default model cleared on re-entry needs its reason visible whichever
         mode the user is in. -->
    {#if ai.aiRestoredModelNote}
      <p class="ob-fine err">{ai.aiRestoredModelNote}</p>
    {/if}
    <AiSetup {ai} aiEnabled={f.aiFeatures} onToggleAi={() => flow.toggleFeature("aiFeatures")} />
  {/if}
</div>

<div class="ob-foot">
  <hr class="ob-rule" />
  <div class="ob-acts">
    {#if aiBlocked}
      <!-- The only rule this screen enforces. Enabled-but-unconfigured is a
           silent failure, so BOTH exits are held until it is resolved either
           way. The tabs stay live — the section that fixes it is one click. -->
      <span class="ob-blocked spacer">
        Back held · AI features are on, with nothing to run them
      </span>
      {#if active !== "ai"}
        <button class="ob-btn" type="button" onclick={() => (active = "ai")}>Add a key</button>
      {/if}
      <button class="ob-btn" type="button" onclick={() => flow.toggleFeature("aiFeatures")}>
        Turn AI features off
      </button>
    {:else}
      <span class="ob-fine spacer">{SECTIONS[at].note}</span>
      {#if at === 0}
        <button class="ob-btn ghost" onclick={onBack}>← Back to your settings</button>
      {:else}
        <button class="ob-btn ghost" onclick={() => (active = SECTIONS[at - 1].id)}>
          ← {SECTIONS[at - 1].label}
        </button>
      {/if}
      {#if at === SECTIONS.length - 1}
        <button class="ob-btn primary" onclick={onContinue}>Back to your settings&nbsp; →</button>
      {:else}
        <button class="ob-btn primary" onclick={() => (active = SECTIONS[at + 1].id)}>
          {SECTIONS[at + 1].label}&nbsp; →
        </button>
      {/if}
    {/if}
  </div>
</div>

<style>
  /* The tab strip that replaced the rail: it names every section, says where
     you are, and costs no width. */
  .tabs {
    flex: none;
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--app-border);
    margin-bottom: 18px;
  }
  .tab {
    font: inherit;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    padding: 8px 14px 9px;
    cursor: pointer;
    white-space: nowrap;
  }
  .tab:hover {
    color: var(--app-text);
  }
  .tab.on {
    color: var(--app-text-strong);
    border-bottom-color: var(--app-accent);
  }
  .tab:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .count {
    margin-left: auto;
    align-self: center;
    font-size: var(--text-xs);
    letter-spacing: 0.14em;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  .scene {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    /* `safe center` centres a section that fits (so a short one — AI features
       before a provider is picked — is not left sitting on a hole) and falls
       back to top-aligned the moment it does not, so a tall section scrolls
       from its first line instead of losing its head. */
    justify-content: safe center;
    /* Room for the gutter bleeds (the audio bracket, a peeked row), cancelled
       so the content box stays where it was. Without it the pane scrolls
       sideways and eats the first character of every line. */
    padding-inline: var(--ob-bleed);
    margin-inline: calc(-1 * var(--ob-bleed));
    gap: 14px;
  }
  .ctx {
    margin: 0;
    /* One status sentence, not prose: `.ob-fine`'s 64ch measure wrapped it to
       two lines on a 1032px-wide screen, and those 19px were exactly what the
       section was short of when the budget escape prints its shortfall. */
    max-width: none;
  }
  .link {
    font: inherit;
    color: var(--app-accent);
    background: transparent;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .gate {
    align-self: flex-start;
  }
  .err {
    color: var(--app-danger);
    margin: 0;
  }
  .spacer {
    margin-right: auto;
  }
</style>
