<script lang="ts">
  // The 3-step guided wizard (Screen 3 of docs/triggers/mockups/final/DESIGN.md):
  // 01 Condition → 02 Prompt → 03 Review. Visited steps are clickable, forward
  // jumps past unvisited steps are not. Create lands on step 1, Import on step 2
  // (the prompt is what needs review), Edit on Review with all steps unlocked.
  // Slice 6 adds a Template step 0 (TemplateGallery) for plain creates (no
  // presetCond): a pick prefills everything and lands on Review with a
  // removable "from template" chip; Start from scratch begins at 01.
  import { tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  // Plain (unscoped) styles, every selector prefixed with the .wiz-panel root —
  // extracted to keep this file under the repo's 800-line ceiling.
  import "./wizard.css";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import IconCheck from "~icons/lucide/check";
  import IconChevronLeft from "~icons/lucide/chevron-left";
  import IconChevronRight from "~icons/lucide/chevron-right";
  import IconTriangleAlert from "~icons/lucide/triangle-alert";
  import type { PrivacyAppCandidateDto } from "$lib/app-privacy-exclusion";
  import {
    CONDITION_SECTIONS,
    DEFAULT_AWAY_GAP_MINUTES,
    DEFAULT_COOLDOWN_MINUTES,
    DEFAULT_MIN_MEETING_MINUTES,
    STARTERS,
    WEEKDAY_ORDER,
    advRows,
    createTrigger,
    scheduleLabel,
    updateTrigger,
    type ConditionType,
    type ScheduleWeekday,
    type TriggerCondition,
    type TriggerDefinition,
    type TriggerDraft,
  } from "$lib/triggers/api";
  import { CONDITION_ICON } from "$lib/triggers/condition-icons";
  import { shareTriggerJson } from "$lib/triggers/share";
  import TemplateGallery from "$lib/triggers/TemplateGallery.svelte";
  import { templatePrefill, type TriggerTemplate } from "$lib/triggers/templates";

  interface Props {
    mode: "create" | "edit" | "import";
    presetCond?: ConditionType;
    editing?: TriggerDefinition | null;
    imported?: TriggerDraft | null;
    providerReady: boolean;
    oncancel: () => void;
    onsaved: (trigger: TriggerDefinition) => void;
    onsetupprovider: () => void;
  }

  let {
    mode,
    presetCond,
    editing = null,
    imported = null,
    providerReady,
    oncancel,
    onsaved,
    onsetupprovider,
  }: Props = $props();

  // ── One-shot init from props (the page keys this component per open, so
  // deliberately NOT reactive — hence the untrack). ─────────────────────────
  const seed: TriggerDraft | null = untrack(() =>
    mode === "edit" && editing
      ? {
          name: editing.name,
          condition: editing.condition,
          prompt: editing.prompt,
          ...(editing.cooldownMinutes !== undefined
            ? { cooldownMinutes: editing.cooldownMinutes }
            : {}),
        }
      : mode === "import"
        ? imported
        : null,
  );

  const initialCond: ConditionType = untrack(
    () => seed?.condition.type ?? presetCond ?? "meeting_ends",
  );

  // Plain creates (no condition preset from an add-row) start on the template
  // gallery — step 0. Edit/import/preset creates keep their existing landing.
  const hasGallery: boolean = untrack(() => mode === "create" && presetCond === undefined);

  let step = $state(hasGallery ? 0 : 1);
  let maxStep = $state(hasGallery ? 0 : 1);
  let cond = $state<ConditionType>(initialCond);
  let name = $state(seed?.name ?? "");
  let prompt = $state(seed ? seed.prompt : STARTERS[initialCond]);
  let dirty = $state(seed ? seed.prompt !== STARTERS[initialCond] : false);
  let appBundleId = $state(
    seed?.condition.type === "app_opened" ? seed.condition.bundleId : "",
  );
  let appName = $state(seed?.condition.type === "app_opened" ? seed.condition.appName : "");
  let time = $state(seed?.condition.type === "schedule" ? seed.condition.time : "18:00");
  // Multi-select weekday set (DESIGN.md Screen 3: Mon–Sun chips). All 7 on
  // maps to the wire's `daily` cadence, any subset to `weekly` + the set.
  // New schedule triggers default to weekdays (Mon–Fri, the mockup's preview).
  let schedDays = $state<ScheduleWeekday[]>(
    seed?.condition.type === "schedule"
      ? seed.condition.cadence === "daily"
        ? [...WEEKDAY_ORDER]
        : [...(seed.condition.weekdays ?? [])]
      : WEEKDAY_ORDER.slice(0, 5),
  );
  let adv = $state({
    minlen:
      seed?.condition.type === "meeting_ends"
        ? (seed.condition.minMeetingMinutes ?? DEFAULT_MIN_MEETING_MINUTES)
        : DEFAULT_MIN_MEETING_MINUTES,
    awaygap:
      seed?.condition.type === "app_opened"
        ? (seed.condition.awayGapMinutes ?? DEFAULT_AWAY_GAP_MINUTES)
        : DEFAULT_AWAY_GAP_MINUTES,
    cooldown: seed?.cooldownMinutes ?? DEFAULT_COOLDOWN_MINUTES,
  });
  let advOpen = $state(false);
  let nameError = $state(false);
  let saveError = $state<string | null>(null);
  let saving = $state(false);
  let previewExpanded = $state(false);
  let shareState = $state<"idle" | "copied" | "failed">("idle");
  let nameInput = $state<HTMLInputElement | null>(null);
  let panelEl = $state<HTMLDivElement | null>(null);
  /** Template identity for the Review chip — values survive its removal. */
  let template = $state<TriggerTemplate | null>(null);

  // Edit lands on Review with every step unlocked; Import lands on the prompt.
  untrack(() => {
    if (mode === "edit") {
      step = 3;
      maxStep = 3;
    } else if (mode === "import") {
      step = 2;
      maxStep = 2;
    }
  });

  const crumb = $derived(
    mode === "edit" ? `edit · ${editing?.name ?? ""}` : mode === "import" ? "import trigger" : "new trigger",
  );

  // Creation-time Provider Gate: creating is blocked (backend enforces too);
  // editing an existing trigger stays allowed while unconfigured.
  const createBlocked = $derived(mode !== "edit" && !providerReady);

  // ── App picker (the privacy-exclusions installed-app source, reused) ──────
  let appCandidates = $state<PrivacyAppCandidateDto[]>([]);
  let appsLoaded = $state(false);
  $effect(() => {
    void (async () => {
      try {
        const all = await invoke<PrivacyAppCandidateDto[]>("list_privacy_app_candidates");
        appCandidates = all.filter((c) => c.bundleId !== "com.shaikzeeshan.mnema");
      } catch {
        appCandidates = [];
      } finally {
        appsLoaded = true;
      }
    })();
  });
  // Options = candidates, plus the edited trigger's app if it's not installed
  // anymore (so editing never silently swaps the app).
  const appOptions = $derived.by(() => {
    if (appBundleId && !appCandidates.some((c) => c.bundleId === appBundleId)) {
      return [{ bundleId: appBundleId, displayName: appName || appBundleId }, ...appCandidates];
    }
    return appCandidates;
  });
  // Default the selection to the first option once loaded.
  $effect(() => {
    if (appsLoaded && !appBundleId && appOptions.length > 0) {
      appBundleId = appOptions[0].bundleId;
      appName = appOptions[0].displayName;
    }
  });
  function onAppPick(event: Event): void {
    const bundleId = (event.currentTarget as HTMLSelectElement).value;
    const picked = appOptions.find((c) => c.bundleId === bundleId);
    if (picked) {
      appBundleId = picked.bundleId;
      appName = picked.displayName;
    }
  }

  // Meeting Ends disclosure (ADR 0057 amendment 2026-07-21): the meeting-URL
  // probe obeys the capture browser-URL setting, so with it off, browser
  // meetings can't be detected — say so where the trigger is created.
  let browserUrlOff = $state(false);
  $effect(() => {
    void (async () => {
      try {
        const settings = await invoke<{
          metadata: { enabled: boolean; browserUrlMode: "off" | "sanitized" | "full" };
        }>("get_recording_settings");
        browserUrlOff = !settings.metadata.enabled || settings.metadata.browserUrlMode === "off";
      } catch {
        browserUrlOff = false;
      }
    })();
  });

  // ── Step navigation ───────────────────────────────────────────────────────
  function goStep(n: number): void {
    step = n;
    maxStep = Math.max(maxStep, n);
    if (n === 3) previewExpanded = false;
  }

  const STEP_TABS = $derived([
    ...(hasGallery ? [{ n: 0, label: "Template" }] : []),
    { n: 1, label: "Condition" },
    { n: 2, label: "Prompt" },
    { n: 3, label: "Review" },
  ]);

  // ── Step 0: template gallery ──────────────────────────────────────────────
  // Picking a card prefills everything and lands on Review; the picked card's
  // button leaves the DOM, so focus moves to the Name field (a11y).
  function applyTemplate(tpl: TriggerTemplate): void {
    template = tpl;
    const fill = templatePrefill(tpl);
    cond = fill.cond;
    name = fill.name;
    prompt = fill.prompt;
    dirty = fill.prompt !== STARTERS[fill.cond];
    if (fill.appBundleId !== undefined) appBundleId = fill.appBundleId;
    if (fill.appName !== undefined) appName = fill.appName;
    if (fill.awayGap !== undefined) adv.awaygap = fill.awayGap;
    if (fill.minLen !== undefined) adv.minlen = fill.minLen;
    if (fill.time !== undefined) time = fill.time;
    if (fill.schedDays !== undefined) schedDays = fill.schedDays;
    goStep(3);
    void tick().then(() => nameInput?.focus());
  }

  // Removing the chip keeps every prefilled value — only the identity clears.
  function clearTemplate(): void {
    template = null;
    void tick().then(() => nameInput?.focus());
  }

  function startScratch(): void {
    template = null;
    goStep(1);
    void tick().then(() =>
      panelEl?.querySelector<HTMLButtonElement>(".cond-card")?.focus(),
    );
  }

  // ── Step 1: condition ─────────────────────────────────────────────────────
  function selectCond(next: ConditionType): void {
    cond = next;
    // Switching condition swaps the starter only while the prompt is unedited.
    if (!dirty) prompt = STARTERS[next];
  }

  // Mon…Sun toggle chips ("monday" → "Mon").
  const DAY_CHIPS = WEEKDAY_ORDER.map((day) => ({
    day,
    label: `${day.charAt(0).toUpperCase()}${day.slice(1, 3)}`,
  }));
  function toggleDay(day: ScheduleWeekday): void {
    schedDays = schedDays.includes(day)
      ? schedDays.filter((d) => d !== day)
      : [...schedDays, day];
  }
  // Backend rule mirrored: "a weekly schedule needs at least one weekday".
  const schedInvalid = $derived(cond === "schedule" && schedDays.length === 0);
  const schedCondition = $derived.by(
    (): Extract<TriggerCondition, { type: "schedule" }> =>
      schedDays.length === 7
        ? { type: "schedule", cadence: "daily", time: time || "18:00" }
        : {
            type: "schedule",
            cadence: "weekly",
            time: time || "18:00",
            weekdays: WEEKDAY_ORDER.filter((day) => schedDays.includes(day)),
          },
  );
  const schedPreview = $derived(`Runs ${scheduleLabel(schedCondition)}.`);

  // ── Step 2: prompt ────────────────────────────────────────────────────────
  function onPromptInput(event: Event): void {
    prompt = (event.currentTarget as HTMLTextAreaElement).value;
    dirty = prompt !== STARTERS[cond];
  }
  function resetPrompt(): void {
    prompt = STARTERS[cond];
    dirty = false;
  }

  // ── Step 3: review ────────────────────────────────────────────────────────
  const condEcho = $derived.by(() => {
    if (cond === "meeting_ends") {
      return {
        main: "Meeting Ends",
        sub: `a conferencing app releases the mic after holding it ≥${adv.minlen} min`,
      };
    }
    if (cond === "app_opened") {
      return {
        main: `App Opened — ${appName || "pick an app"}`,
        sub: `fires on a fresh session, after ${adv.awaygap} min away`,
      };
    }
    return {
      main: "Schedule",
      sub: `runs ${scheduleLabel(schedCondition)}`,
    };
  });

  const ADV_ROWS = $derived(advRows(cond));
  const allDefaults = $derived(
    adv.minlen === DEFAULT_MIN_MEETING_MINUTES &&
      adv.awaygap === DEFAULT_AWAY_GAP_MINUTES &&
      adv.cooldown === DEFAULT_COOLDOWN_MINUTES,
  );
  function bumpAdv(key: "minlen" | "awaygap" | "cooldown", delta: number, min: number, max: number): void {
    adv[key] = Math.min(max, Math.max(min, adv[key] + delta));
  }
  // Spinbutton keyboard support for the Advanced steppers.
  function advKeydown(
    event: KeyboardEvent,
    row: { key: "minlen" | "awaygap" | "cooldown"; step: number; min: number; max: number },
  ): void {
    if (event.key === "ArrowUp") {
      event.preventDefault();
      bumpAdv(row.key, row.step, row.min, row.max);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      bumpAdv(row.key, -row.step, row.min, row.max);
    }
  }

  // ── Save ──────────────────────────────────────────────────────────────────
  function buildCondition(): TriggerCondition {
    if (cond === "meeting_ends") {
      return {
        type: "meeting_ends",
        ...(adv.minlen !== DEFAULT_MIN_MEETING_MINUTES ? { minMeetingMinutes: adv.minlen } : {}),
      };
    }
    if (cond === "app_opened") {
      return {
        type: "app_opened",
        bundleId: appBundleId,
        appName,
        ...(adv.awaygap !== DEFAULT_AWAY_GAP_MINUTES ? { awayGapMinutes: adv.awaygap } : {}),
      };
    }
    return schedCondition;
  }

  async function save(): Promise<void> {
    const trimmed = name.trim();
    if (!trimmed) {
      nameError = true;
      nameInput?.focus();
      return;
    }
    saving = true;
    saveError = null;
    const condition = buildCondition();
    const cooldownMinutes =
      adv.cooldown !== DEFAULT_COOLDOWN_MINUTES ? adv.cooldown : undefined;
    try {
      let saved: TriggerDefinition;
      if (mode === "edit" && editing) {
        // Preserve id + enabled state + run history; only the edited fields move.
        const payload: TriggerDefinition = {
          ...editing,
          name: trimmed,
          condition,
          prompt,
        };
        delete payload.cooldownMinutes;
        if (cooldownMinutes !== undefined) payload.cooldownMinutes = cooldownMinutes;
        saved = await updateTrigger(payload);
      } else {
        saved = await createTrigger({
          name: trimmed,
          condition,
          prompt,
          ...(cooldownMinutes !== undefined ? { cooldownMinutes } : {}),
        });
      }
      onsaved(saved);
    } catch (error) {
      saveError = String(error);
    } finally {
      saving = false;
    }
  }

  async function shareJson(): Promise<void> {
    const shareable: TriggerDefinition = {
      id: editing?.id ?? "",
      name: name.trim() || "Untitled Trigger",
      condition: buildCondition(),
      prompt,
      enabled: true,
      version: 1,
    };
    if (adv.cooldown !== DEFAULT_COOLDOWN_MINUTES) shareable.cooldownMinutes = adv.cooldown;
    try {
      await writeText(shareTriggerJson(shareable));
      shareState = "copied";
    } catch {
      // Clipboard unavailable — say so instead of flashing a false success.
      shareState = "failed";
    }
    setTimeout(() => (shareState = "idle"), 1400);
  }

  function onNext(): void {
    if (step < 3) {
      goStep(step + 1);
      return;
    }
    void save();
  }
</script>

<div class="wiz-panel" class:at-gallery={step === 0} bind:this={panelEl}>
  <nav class="wiz-crumb">
    <button class="back" type="button" onclick={oncancel}>triggers</button>
    <span class="sep">/</span>
    <span class="here">{crumb}</span>
  </nav>

  {#if mode === "import"}
    <div class="warn-banner" role="status">
      <span class="glyph" aria-hidden="true"><IconTriangleAlert /></span>
      <span>
        Imported — review this prompt before saving. It will run automatically, with read-only
        access to your capture history, whenever the condition fires.
      </span>
    </div>
  {/if}

  {#if createBlocked}
    <div class="warn-banner" role="status">
      <span class="glyph" aria-hidden="true"><IconTriangleAlert /></span>
      <span>
        Triggers need an AI provider before they can be created — nothing can run without one.
        <button type="button" class="banner-link" onclick={onsetupprovider}>Set up provider</button>
      </span>
    </div>
  {/if}

  <div class="steps">
    {#each STEP_TABS as tab, index (tab.n)}
      {#if index > 0}
        <span class="step-rule" class:done={tab.n <= step}></span>
      {/if}
      <button
        class="step-tab"
        class:current={tab.n === step}
        class:done={tab.n !== step && tab.n <= maxStep}
        type="button"
        disabled={tab.n > maxStep}
        onclick={() => {
          if (tab.n !== step && tab.n <= maxStep) goStep(tab.n);
        }}
      >
        <span class="num">{tab.n === 0 ? (step > 0 ? "✓" : "✦") : `0${tab.n}`}</span>{tab.label}
      </button>
    {/each}
  </div>

  <!-- STEP 0 — template gallery -->
  {#if step === 0}
    <TemplateGallery onpick={applyTemplate} onscratch={startScratch} />
  {/if}

  <!-- STEP 1 — condition -->
  {#if step === 1}
    <div class="wiz-step">
      <p class="wiz-lead">
        <span class="q">When should this trigger run?</span>
        <span class="hint">
          Conditions are situations Mnema can detect on its own — pick one; you can't define new
          kinds.
        </span>
      </p>
      <div class="cond-pick" role="group" aria-label="Condition">
        {#each CONDITION_SECTIONS as card (card.cond)}
          {@const Icon = CONDITION_ICON[card.cond]}
          <button
            type="button"
            class="cond-card"
            class:selected={cond === card.cond}
            aria-pressed={cond === card.cond}
            onclick={() => selectCond(card.cond)}
          >
            <span class="radio" aria-hidden="true"></span>
            <span class="cond-name"><Icon aria-hidden="true" />{card.title}</span>
            <span class="cond-desc">
              {#if card.cond === "meeting_ends"}
                A conferencing app (Zoom, Teams, Meet in a browser…) held your mic for at least 5
                minutes, then released it and stayed quiet for ~2 minutes.
              {:else if card.cond === "app_opened"}
                A chosen app comes to the front after you've been away from it for 30 minutes or
                more — a fresh working session, not window switching.
              {:else}
                At a fixed local time on the days you pick — good for daily wrap-ups and weekly
                reviews.
              {/if}
            </span>
          </button>
          {#if cond === card.cond}
            <div class="cond-params">
              {#if card.cond === "meeting_ends"}
                <p class="params-lbl">How Mnema detects it</p>
                <p class="hint no-top">
                  Detection is automatic — nothing to configure. Mnema watches which app holds the
                  microphone (the macOS "orange dot" signal); a browser counts when a meeting URL
                  was seen during the call. Drop/rejoin gaps are absorbed. Privacy-excluded
                  browsers are never checked.
                </p>
                {#if browserUrlOff}
                  <p class="hint browser-url-off" role="note">
                    Browser-URL collection is turned off in Settings, so meetings in a browser
                    (Google Meet, browser Zoom…) won't be detected — only conferencing apps will.
                  </p>
                {/if}
              {:else if card.cond === "app_opened"}
                <p class="params-lbl">Which app?</p>
                <div class="field">
                  <select
                    class="text-input"
                    aria-label="App"
                    value={appBundleId}
                    onchange={onAppPick}
                  >
                    {#if !appsLoaded}
                      <option value="">Loading apps…</option>
                    {:else if appOptions.length === 0}
                      <option value="">No apps found</option>
                    {/if}
                    {#each appOptions as candidate (candidate.bundleId)}
                      <option value={candidate.bundleId}>{candidate.displayName}</option>
                    {/each}
                  </select>
                </div>
                <p class="hint">
                  Any moment the app is frontmost resets the 30-minute clock, so rapid switching
                  never fires it.
                </p>
              {:else}
                <p class="params-lbl">When?</p>
                <div class="sched-grid">
                  <div class="field">
                    <label for="sched-time">Time</label>
                    <input class="text-input" id="sched-time" type="time" bind:value={time} />
                  </div>
                  <div class="field">
                    <span class="field-lbl">Days</span>
                    <div class="day-chips" role="group" aria-label="Days">
                      {#each DAY_CHIPS as chip (chip.day)}
                        <button
                          type="button"
                          class="day-chip"
                          class:on={schedDays.includes(chip.day)}
                          aria-pressed={schedDays.includes(chip.day)}
                          onclick={() => toggleDay(chip.day)}
                        >{chip.label}</button>
                      {/each}
                    </div>
                  </div>
                </div>
                {#if schedInvalid}
                  <p class="sched-err" role="alert">A schedule needs at least one day.</p>
                {:else}
                  <p class="sched-preview">{schedPreview}</p>
                {/if}
              {/if}
            </div>
          {/if}
        {/each}
      </div>
    </div>
  {/if}

  <!-- STEP 2 — prompt -->
  {#if step === 2}
    <div class="wiz-step">
      <p class="wiz-lead">
        <span class="q">What should Mnema do when it fires?</span>
        <span class="hint">
          Write it like you'd brief a person. We started you off with a template for this
          condition.
        </span>
      </p>
      <section class="prompt-pane" aria-label="Prompt">
        <div class="prompt-head">
          <span class="lbl">Prompt</span>
          <span class="chip">{dirty ? "edited" : "starter template"}</span>
          <button class="quiet-action" type="button" onclick={resetPrompt}>Reset</button>
        </div>
        <textarea
          class="prompt-editor"
          spellcheck="false"
          value={prompt}
          oninput={onPromptInput}
        ></textarea>
        <div class="prompt-foot">
          <span>plain prose, no variables — Mnema adds context automatically</span>
          <span>{prompt.length} chars</span>
        </div>
      </section>
    </div>
  {/if}

  <!-- STEP 3 — review -->
  {#if step === 3}
    <div class="wiz-step">
      {#if template !== null}
        {@const ChipIcon = CONDITION_ICON[template.condition.type]}
        <span class="tpl-chip">
          <ChipIcon aria-hidden="true" />
          from template · {template.name}
          <button
            type="button"
            class="x"
            aria-label="Remove template"
            title="Clear the template — keeps everything it filled in"
            onclick={clearTemplate}
          >✕</button>
        </span>
      {/if}
      <p class="wiz-lead">
        <span class="q">{mode === "edit" ? "Review and save." : "Review and create."}</span>
        <span class="hint">
          {#if template !== null}
            The template filled everything in — steps 01 and 02 are unlocked if you want to
            change the condition or reword the prompt.
          {:else}
            Name it, check the pieces, tune Advanced only if you must — the defaults are right
            for almost everyone.
          {/if}
        </span>
      </p>
      <div class="review-card">
        <div class="review-sec">
          <p class="review-lbl">Name</p>
          <div class="field">
            <input
              class="text-input"
              class:invalid={nameError}
              type="text"
              placeholder="e.g. Meeting Recap"
              bind:this={nameInput}
              bind:value={name}
              oninput={() => {
                if (name.trim()) nameError = false;
              }}
            />
          </div>
          {#if nameError}
            <p class="name-err">Give it a name — it labels runs in your chat rail and notifications.</p>
          {/if}
        </div>

        <div class="review-sec">
          <p class="review-lbl">Condition</p>
          <div class="review-echo">
            <span class="main">{condEcho.main}</span>
            <span class="sub">{condEcho.sub}</span>
          </div>
        </div>

        <div class="review-sec">
          <p class="review-lbl">Prompt</p>
          <div class="prompt-preview" class:expanded={previewExpanded}>{prompt}</div>
          <button
            class="preview-more"
            type="button"
            onclick={() => (previewExpanded = !previewExpanded)}
          >{previewExpanded ? "show less" : "show all"}</button>
        </div>

        <div class="review-sec">
          <p class="review-lbl">What Mnema adds automatically</p>
          <ul class="auto-list">
            <li><span class="tick" aria-hidden="true"><IconCheck /></span> the firing window — condition, time span, app</li>
            <li><span class="tick" aria-hidden="true"><IconCheck /></span> your User Context — what Mnema has learned about your work</li>
            <li><span class="tick" aria-hidden="true"><IconCheck /></span> speaker identity — which voice in the audio is you</li>
            <li><span class="tick" aria-hidden="true"><IconCheck /></span> previous runs of this trigger, so results compound</li>
          </ul>
          <p class="auto-note">
            Assembled on every run. Nothing to configure — want a generic result? Just say so in
            the prompt.
          </p>
        </div>

        <div class="review-sec">
          <details class="disclosure" bind:open={advOpen}>
            <summary>
              <span class="caret" aria-hidden="true"><IconChevronRight /></span> Advanced
              <span class="default-note">{allDefaults ? "all defaults" : "modified"}</span>
            </summary>
            <div class="disc-body">
              {#each ADV_ROWS as row (row.key)}
                <div class="stepper-row">
                  <span class="stepper-lbl">{row.label}</span>
                  <span
                    class="stepper"
                    role="spinbutton"
                    tabindex="0"
                    aria-label={row.label}
                    aria-valuenow={adv[row.key]}
                    aria-valuemin={row.min}
                    aria-valuemax={row.max}
                    aria-valuetext={`${adv[row.key]} min`}
                    onkeydown={(event) => advKeydown(event, row)}
                  >
                    <button
                      type="button"
                      tabindex="-1"
                      disabled={adv[row.key] <= row.min}
                      aria-label={`Decrease ${row.label}`}
                      onclick={() => bumpAdv(row.key, -row.step, row.min, row.max)}
                    >−</button>
                    <span class="val">{adv[row.key]} min</span>
                    <button
                      type="button"
                      tabindex="-1"
                      disabled={adv[row.key] >= row.max}
                      aria-label={`Increase ${row.label}`}
                      onclick={() => bumpAdv(row.key, row.step, row.min, row.max)}
                    >+</button>
                  </span>
                </div>
              {/each}
            </div>
          </details>
        </div>
      </div>
      {#if saveError}
        <p class="save-err" role="alert">{saveError}</p>
      {/if}
    </div>
  {/if}

  {#if step > 0}
  <div class="wiz-nav">
    {#if step > (hasGallery ? 0 : 1)}
      <button class="btn" type="button" onclick={() => goStep(step - 1)}>
        <IconChevronLeft aria-hidden="true" /> Back
      </button>
    {/if}
    <span class="spacer"></span>
    {#if step === 3}
      <span class="note">saved to triggers.json — runs stay on this Mac</span>
      <button
        class="btn btn--ghost"
        class:btn--fail={shareState === "failed"}
        type="button"
        title="Copies Trigger JSON — never carries provider or model config"
        onclick={() => void shareJson()}
      >
        {#if shareState === "copied"}
          copied <IconCheck aria-hidden="true" />
        {:else if shareState === "failed"}
          copy failed
        {:else}
          Share as JSON
        {/if}
      </button>
    {/if}
    <button
      class="btn btn--accent"
      type="button"
      disabled={saving || schedInvalid || (step === 3 && createBlocked)}
      onclick={onNext}
    >
      {#if step < 3}
        Next <IconChevronRight aria-hidden="true" />
      {:else}
        {saving ? "Saving…" : mode === "edit" ? "Save Changes" : "Create Trigger"}
      {/if}
    </button>
  </div>
  {/if}
</div>
