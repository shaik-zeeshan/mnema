<!--
  Screen 5 / 8 — Change settings (issue #195, slice 8).

  The flow's ONE deliberately dense screen: every detail demoted off the other
  seven lands here, in four sections — Capture sources · Processing · Storage ·
  AI features. Provider names, model identifiers, per-row byte sizes and the
  retention reasoning live here and only here; that is what lets *Your settings*
  be eight short rows.

  Two rules this screen carries:
   · `flow.toggleFeature(id)` is the ONLY way to flip a feature — it runs
     `applyToggle`, whose cascades run BOTH directions.
   · The single gate: AI features may not be left on with no credentials
     (enabled-but-unconfigured is a silent failure). Everything else is free.
     Deepgram is never selectable here — cloud transcription is Settings-only,
     behind its own consent gate (ADR 0047).

  Ported from `docs/onboarding/mockups/chosen-cinematic-rewind.html` 1205-1348
  (`chosen-shots/s07-dark.png`), in `var(--app-*)` tokens.
-->
<script lang="ts">
  import Segmented from "$lib/components/Segmented.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import RetentionPicker from "$lib/components/RetentionPicker.svelte";
  import ModelPickerMenu from "$lib/insights/ModelPickerMenu.svelte";
  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    fpsToIntervalS,
    intervalSToFps,
  } from "$lib/components/capture-rate";
  import { estimateDailyStorageMb } from "$lib/onboarding/disk-estimate";
  import {
    featureLockReason,
    featureToggleDisabled,
    systemAudioNeedsRequest,
    type FeatureId,
  } from "$lib/onboarding/feature-rules";
  import { resolveSetup, workListBytes } from "$lib/onboarding/resolve-setup";
  import { SELECTABLE_OCR_PROVIDERS } from "../onboarding-mapping";
  import { OS_MANAGED_OPTION_VALUE } from "../onboarding-models.svelte";
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
    { id: "sources", label: "Capture sources" },
    { id: "processing", label: "Processing" },
    { id: "storage", label: "Storage" },
    { id: "ai", label: "AI features" },
  ] as const;
  let pane = $state<HTMLDivElement | null>(null);
  let active = $state<string>("sources");

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

  // ── Per-row byte sizes — read from the work-list, never pasted ────────────
  // Re-resolved against the LIVE feature state so a toggle here moves the sizes
  // immediately (`flow.resolved.workList` is only rebuilt on the Permissions
  // exit).
  const workList = $derived(
    resolveSetup(
      f.permissions,
      {
        speakerAnalysis: c.selectedSpeakerModel?.available ?? false,
        whisperBase: c.selectedTranscriptionModel?.available ?? false,
        semanticSearch: c.selectedSemanticSearchModel?.available ?? false,
      },
      { features: f, models: flow.resolved?.models, excludedApps: flow.resolved?.excludedApps ?? [] },
    ).workList,
  );
  function workItem(feature: FeatureId) {
    return workList.find((item) => item.feature === feature) ?? null;
  }
  /**
   * "419 MB to download" / "already on this Mac" for a feature's model. The
   * work-list is the first source; a model the resolver does not manage (a
   * non-default pick) falls back to its own manifest figure.
   */
  function downloadNote(
    feature: FeatureId,
    fallbackBytes: number | null,
    installed: boolean,
  ): string {
    const item = workItem(feature);
    if (item) return `${formatSize(item.bytes)} to download`;
    if (installed) return "already on this Mac";
    return fallbackBytes ? `${formatSize(fallbackBytes)} to download` : "no download";
  }
  const totalBytes = $derived(workListBytes(workList));
  const semanticItem = $derived(workItem("semanticSearch"));
  const semanticShare = $derived(
    semanticItem && totalBytes > 0 ? Math.round((semanticItem.bytes / totalBytes) * 100) : 0,
  );

  // ponytail: decimal MB/GB, not the 1024-based `formatBytes` — every figure in
  // the plan and the model manifests is SI (Whisper base 147,951,465 = 148 MB).
  function formatSize(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
    return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
  }

  // ── Provider pickers ──────────────────────────────────────────────────────
  const ocrProviderOptions = $derived.by(() => {
    const live = (c.ocrModelStatus?.providers ?? []).filter((p) =>
      SELECTABLE_OCR_PROVIDERS.includes(p.provider),
    );
    return live.length > 0
      ? live.map((p) => ({ value: p.provider as string, label: p.displayName }))
      : [
          { value: "apple_vision", label: "Apple Vision" },
          { value: "tesseract", label: "Tesseract" },
        ];
  });

  // ADR 0047: Deepgram is filtered out — cloud transcription never appears in
  // onboarding. (`chooseTranscriptionProvider` refuses it defensively too.)
  const transcriptionProviderOptions = $derived.by(() => {
    const live = (c.transcriptionModelStatus?.providers ?? []).filter(
      (p) => p.provider !== "deepgram",
    );
    return live.length > 0
      ? live.map((p) => ({ value: p.provider as string, label: p.displayName }))
      : [
          { value: "local_whisper", label: "Local Whisper" },
          { value: "apple_speech_on_device", label: "Apple Speech" },
          { value: "parakeet", label: "Parakeet" },
        ];
  });

  // Model identifiers WITH their real byte sizes — the detail this screen exists
  // to carry. `download.byteSize` is the manifest's own figure.
  const transcriptionModelOptions = $derived(
    c.selectedTranscriptionModels.map((model) => ({
      value: model.modelId ?? OS_MANAGED_OPTION_VALUE,
      label: `${model.displayName} · ${
        model.available
          ? "installed"
          : model.download
            ? formatSize(model.download.byteSize)
            : "managed by macOS"
      }`,
    })),
  );

  // ── Storage ───────────────────────────────────────────────────────────────
  const captureRateOptions = CAPTURE_INTERVAL_LADDER_S.map((s) => ({
    value: String(s),
    label: `${captureIntervalPhrase(s)} · about ${Math.round(estimateDailyStorageMb(s))} MB a day`,
  }));
  const captureRateValue = $derived(String(fpsToIntervalS(c.draftFrameRate)));

  // ── The one gate: AI on with nothing to run it ────────────────────────────
  const aiBlocked = $derived(f.aiFeatures && !ai.aiConfigReady);

  // Configuring a provider ENABLES the feature — otherwise the user pastes a key
  // and nothing uses it. Fires only on the false→true transition, so turning the
  // row off afterwards with a working config is not undone.
  let wasReady = false;
  $effect(() => {
    const ready = ai.aiConfigReady;
    if (ready && !wasReady && !flow.features.aiFeatures) flow.toggleFeature("aiFeatures");
    wasReady = ready;
  });

  const localKinds = $derived(ai.AI_PROVIDER_KINDS.filter((k) => !ai.isCloudAiProviderKind(k)));
  const cloudKinds = $derived(ai.AI_PROVIDER_KINDS.filter((k) => ai.isCloudAiProviderKind(k)));
  const keySaved = $derived(ai.aiRuntime.aiProviderKeySavedByProvider);
  const keySaving = $derived(ai.aiRuntime.aiProviderKeySavingProvider);
  const keyInputs = $derived(ai.aiRuntime.aiProviderKeyInputs);
  const keyErrors = $derived(ai.aiRuntime.aiProviderKeyErrors);
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
    <!-- ── Capture sources ─────────────────────────────────────────────── -->
    <section class="sec" id="cz-sources">
      <span class="ob-m">Capture sources</span>

      <div class="cz-row">
        <div>
          <div class="t">Screen capture</div>
          <div class="d" class:warn={!f.permissions.screen}>
            {f.permissions.screen
              ? "Frames of your screen. Everything else is built on top of them."
              : "Screen Recording is not granted — capture stays listed but records nothing."}
          </div>
        </div>
        <Switch
          checked={f.screen}
          ariaLabel="Screen capture"
          onCheckedChange={() => flow.toggleFeature("screen")}
        />
      </div>

      <div class="cz-row" class:locked={featureToggleDisabled(f, "microphone")}>
        <div>
          <div class="t">Microphone</div>
          <div class="d" class:warn={featureToggleDisabled(f, "microphone")}>
            {featureLockReason(f, "microphone") && !f.microphone
              ? "Needs Microphone permission — grant it on the Permissions step."
              : "Turning this off also turns off transcription and who's speaking."}
          </div>
        </div>
        <Switch
          checked={f.microphone}
          disabled={featureToggleDisabled(f, "microphone")}
          ariaLabel="Microphone"
          onCheckedChange={() => flow.toggleFeature("microphone")}
        />
      </div>

      <div class="cz-row">
        <div>
          <div class="t">System audio</div>
          <div class="d">
            What your Mac plays. Excludes Mnema itself and every privacy-listed app.{systemAudioNeedsRequest(
              f,
            )
              ? " macOS can't confirm this grant."
              : ""}
          </div>
        </div>
        <Switch
          checked={f.systemAudio}
          ariaLabel="System audio"
          onCheckedChange={() => flow.toggleFeature("systemAudio")}
        />
      </div>
    </section>

    <!-- ── Processing ──────────────────────────────────────────────────── -->
    <section class="sec" id="cz-processing">
      <span class="ob-m">Processing</span>

      <div class="cz-row">
        <div>
          <div class="t">Read on-screen text</div>
          <div class="d">On-device, nothing uploaded. Apple Vision needs no download.</div>
        </div>
        <div class="ctl">
          <Segmented
            value={c.draftOcrProvider}
            options={ocrProviderOptions}
            ariaLabel="OCR provider"
            onValueChange={(v) => c.chooseOcrProvider(v)}
          />
          <Switch
            checked={f.ocr}
            ariaLabel="Read on-screen text"
            onCheckedChange={() => flow.toggleFeature("ocr")}
          />
        </div>
      </div>

      <div class="cz-row">
        <div>
          <div class="t">Transcription</div>
          <div class="d">
            Runs locally. Larger models are slower and more accurate — {downloadNote(
              "transcription",
              c.selectedTranscriptionModel?.download?.byteSize ?? null,
              c.selectedTranscriptionModel?.available ?? false,
            )}.
          </div>
        </div>
        <div class="ctl">
          <Segmented
            value={c.draftTranscriptionProvider}
            options={transcriptionProviderOptions}
            ariaLabel="Transcription provider"
            onValueChange={(v) => c.chooseTranscriptionProvider(v)}
          />
          <div class="picker">
            <Select
              value={c.draftTranscriptionModelId ?? OS_MANAGED_OPTION_VALUE}
              options={transcriptionModelOptions}
              ariaLabel="Transcription model"
              emptyText="No models"
              onValueChange={(v) => c.chooseTranscriptionModel(v)}
            />
          </div>
          <Switch
            checked={f.transcription}
            ariaLabel="Transcription"
            onCheckedChange={() => flow.toggleFeature("transcription")}
          />
        </div>
      </div>

      <div class="cz-row" class:locked={featureToggleDisabled(f, "speakerSeparation")}>
        <div>
          <div class="t">Who's speaking</div>
          <div class="d" class:warn={featureToggleDisabled(f, "speakerSeparation")}>
            {featureLockReason(f, "speakerSeparation") && !f.speakerSeparation
              ? "Needs transcription on."
              : `Separates voices in a conversation. speakrs pyannote-community-1 + WeSpeaker on CoreML, ${downloadNote("speakerSeparation", null, c.selectedSpeakerModel?.available ?? false)}.`}
          </div>
        </div>
        <Switch
          checked={f.speakerSeparation}
          disabled={featureToggleDisabled(f, "speakerSeparation")}
          ariaLabel="Who's speaking"
          onCheckedChange={() => flow.toggleFeature("speakerSeparation")}
        />
      </div>

      <div class="cz-row">
        <div>
          <div class="t">Semantic Search</div>
          <div class="d">
            Finds by meaning, not just by word. {downloadNote(
              "semanticSearch",
              c.selectedSemanticSearchModel?.approxDownloadBytes ?? null,
              c.selectedSemanticSearchModel?.available ?? false,
            )}{semanticShare > 0
              ? ` — ${semanticShare}% of the whole download`
              : ""}.
          </div>
        </div>
        <div class="ctl">
          <div class="picker wide">
            <Select
              value={c.draftSemanticSearchModelId}
              options={c.semanticSearchModelOptions}
              placeholder="Choose a model"
              ariaLabel="Semantic Search model"
              loading={c.loadingSemanticSearchSupportedModels}
              emptyText="No models"
              onValueChange={(v) => c.chooseSemanticSearchModel(v)}
            />
          </div>
          <Switch
            checked={f.semanticSearch}
            ariaLabel="Semantic Search"
            onCheckedChange={() => flow.toggleFeature("semanticSearch")}
          />
        </div>
      </div>
    </section>

    <!-- ── Storage ─────────────────────────────────────────────────────── -->
    <section class="sec" id="cz-storage">
      <span class="ob-m">Storage</span>

      <div class="cz-row">
        <div>
          <div class="t">How long to keep it</div>
          <div class="d">
            There is no undo, so the app never picks deletion for you — the default keeps
            everything.
          </div>
        </div>
        <RetentionPicker
          value={c.draftRetentionPolicy}
          onValueChange={(v) => (c.draftRetentionPolicy = v)}
        />
      </div>

      <div class="cz-row">
        <div>
          <div class="t">Capture rate</div>
          <div class="d">
            Measured 270 MB a day at every 3 s with pause-on-inactivity on; storage scales
            linearly from there.
          </div>
        </div>
        <div class="picker wide">
          <Select
            value={captureRateValue}
            options={captureRateOptions}
            ariaLabel="Capture rate"
            onValueChange={(v) => (c.draftFrameRate = intervalSToFps(Number(v)))}
          />
        </div>
      </div>

      <div class="cz-row">
        <div>
          <div class="t">Where it is saved</div>
          <div class="d">Set on the Capture &amp; Storage step.</div>
        </div>
        <span class="ob-fine">{c.draftSaveDirectory || "the default folder"}</span>
      </div>
    </section>

    <!-- ── AI features ─────────────────────────────────────────────────── -->
    <section class="sec" id="cz-ai">
      <span class="ob-m">AI features</span>
      <p class="ob-fine lead-note">
        Powers Ask AI, digests and User Context. Off until you pick a provider.
      </p>

      <div class="cz-row">
        <div>
          <div class="t">AI features</div>
          <div class="d">
            {ai.aiConfigMissing ?? "Ready — a default model is configured."}
          </div>
        </div>
        <Switch
          checked={f.aiFeatures}
          ariaLabel="AI features"
          onCheckedChange={() => flow.toggleFeature("aiFeatures")}
        />
      </div>

      <!-- Local engines FIRST: no key, no account, nothing leaves the machine. -->
      <div class="cz-row">
        <div>
          <div class="t">On this Mac</div>
          <div class="d">No key, no account, nothing leaves the machine.</div>
        </div>
        <div class="ctl">
          {#each localKinds as kind (kind)}
            <button
              class="ob-btn sm"
              type="button"
              disabled={ai.aiProviderRemoving}
              onclick={() => ai.addProvider(kind)}
            >
              + {ai.aiProviderKindLabel(kind)}
            </button>
          {/each}
        </div>
      </div>

      <div class="cz-row">
        <div>
          <div class="t">Or a cloud account</div>
          <div class="d">
            Your own key. Prompts and the context around them are sent to that provider.
          </div>
        </div>
        <div class="ctl">
          {#each cloudKinds as kind (kind)}
            <button
              class="ob-btn sm"
              type="button"
              disabled={ai.aiProviderRemoving}
              onclick={() => ai.addProvider(kind)}
            >
              + {ai.aiProviderKindLabel(kind)}
            </button>
          {/each}
        </div>
      </div>

      {#each ai.draftAiProviders as provider (provider.id)}
        <div class="prov">
          <div class="prov-head">
            <span class="t">{ai.aiProviderInstanceLabel(provider)}</span>
            <span class="ob-fine">
              {ai.isCloudAiProviderKind(provider.kind) ? "cloud" : "local"}
            </span>
            {#if keySaved[provider.id]}<span class="saved">✓ key in keychain</span>{/if}
            <button
              class="ob-btn sm"
              type="button"
              disabled={keySaving !== null || ai.aiProviderRemoving}
              onclick={() => ai.removeProvider(provider.id)}
            >
              Remove
            </button>
          </div>

          {#if ai.isCloudAiProviderKind(provider.kind)}
            {#if provider.kind === "openai_compatible"}
              <input
                class="fld"
                autocomplete="off"
                placeholder="https://api.example.com/v1"
                aria-label="Base URL"
                bind:value={provider.baseUrl}
              />
            {/if}
            <div class="prov-key">
              <input
                class="fld"
                type="password"
                autocomplete="off"
                aria-label="API key"
                placeholder={keySaved[provider.id]
                  ? "A key is saved — enter a new one to replace it"
                  : "Paste your provider API key"}
                disabled={keySaving === provider.id}
                bind:value={
                  () => keyInputs[provider.id] ?? "",
                  (v) => ai.aiRuntime.setProviderKeyInput(provider.id, v)
                }
              />
              <button
                class="ob-btn sm"
                type="button"
                disabled={keySaving !== null ||
                  (keyInputs[provider.id] ?? "").trim().length === 0}
                onclick={() => ai.aiRuntime.saveAiProviderKey(provider.id)}
              >
                {keySaving === provider.id ? "Saving…" : "Save key"}
              </button>
            </div>
            {#if keyErrors[provider.id]}
              <p class="ob-fine err">{keyErrors[provider.id]}</p>
            {/if}
          {:else}
            <input
              class="fld"
              autocomplete="off"
              aria-label="Endpoint"
              placeholder={ai.AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? "http://localhost"}
              bind:value={provider.baseUrl}
            />
          {/if}
        </div>
      {/each}

      <div class="cz-row">
        <div>
          <div class="t">Default model</div>
          <div class="d">One merged list across every connected engine.</div>
        </div>
        <div class="picker wide">
          <ModelPickerMenu
            label={ai.aiModelValue || "Choose a default model"}
            title={ai.aiModelValue || "Choose a default model"}
            ariaLabel="Default model"
            block
            placeholder={ai.draftAiDefaultModel === null}
            disabled={ai.draftAiProviders.length === 0}
            modelPool={ai.modelLoader.pool}
            providers={ai.draftAiProviders}
            firstProvider={ai.draftAiDefaultModel?.provider ?? null}
            sentinelLabel="No default model"
            sentinelSelected={ai.draftAiDefaultModel === null}
            selectedProvider={ai.draftAiDefaultModel?.provider ?? null}
            selectedModel={ai.draftAiDefaultModel?.model ?? null}
            loading={ai.modelLoader.loading}
            failures={ai.modelFailureRows}
            onretry={() => void ai.modelLoader.load(ai.modelRetryTargets)}
            bind:open={ai.aiModelOpen}
            onopen={() => void ai.loadModels()}
            onselect={(engine) => (ai.draftAiDefaultModel = engine)}
          />
        </div>
      </div>

      {#if ai.aiUnverifiedNote}
        <p class="ob-fine warn">{ai.aiUnverifiedNote}</p>
      {/if}
      <p class="ob-fine">
        Keys are stored only in the OS keychain — never in a config file.
      </p>
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

  .cz-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 24px;
    align-items: center;
    padding: 10px 0;
  }
  .cz-row .t {
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }
  .cz-row .d {
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-text-subtle);
    margin-top: 3px;
    max-width: 52ch;
  }
  .cz-row.locked .t {
    color: var(--app-text-muted);
  }
  .d.warn {
    color: var(--app-warn);
  }

  /* One line of controls, never wrapping under its own row — a wrapped switch
     reads as belonging to the row below it. */
  .ctl {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: nowrap;
    justify-content: flex-end;
  }
  /* Segment labels are two words; letting them wrap turns a pill into a block. */
  .ctl :global(button) {
    white-space: nowrap;
  }
  .picker {
    width: 185px;
  }
  .picker.wide {
    width: 230px;
  }

  .lead-note {
    margin: 8px 0 4px;
  }

  /* Connected engines: one compact card each. */
  .prov {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-subtle);
    padding: 10px 12px;
    margin: 8px 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .prov-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .prov-head .ob-btn {
    margin-left: auto;
  }
  .saved {
    font-size: var(--text-xs);
    color: var(--app-accent);
  }
  .prov-key {
    display: flex;
    gap: 8px;
  }
  .fld {
    font: inherit;
    font-size: var(--text-sm);
    flex: 1;
    min-width: 0;
    color: var(--app-text);
    background: var(--app-bg);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 7px 10px;
  }
  .fld:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  .err {
    color: var(--app-danger);
  }
  .warn {
    color: var(--app-warn);
  }
  .spacer {
    margin-right: auto;
  }
</style>
