<script lang="ts">
  // The 3-step guided wizard (Screen 3 of docs/triggers/mockups/final/DESIGN.md):
  // 01 Condition → 02 Prompt → 03 Review. Visited steps are clickable, forward
  // jumps past unvisited steps are not. Create lands on step 1, Import on step 2
  // (the prompt is what needs review), Edit on Review with all steps unlocked.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  // Plain (unscoped) styles, every selector prefixed with the .wiz-panel root —
  // extracted to keep this file under the repo's 800-line ceiling.
  import "./wizard.css";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import type { PrivacyAppCandidateDto } from "$lib/app-privacy-exclusion";
  import {
    CONDITION_SECTIONS,
    DEFAULT_AWAY_GAP_MINUTES,
    DEFAULT_COOLDOWN_MINUTES,
    DEFAULT_MIN_MEETING_MINUTES,
    STARTERS,
    createTrigger,
    fmtTime,
    updateTrigger,
    type ConditionType,
    type ScheduleWeekday,
    type TriggerCondition,
    type TriggerDefinition,
    type TriggerDraft,
  } from "$lib/triggers/api";
  import { shareTriggerJson } from "$lib/triggers/share";

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

  let step = $state(1);
  let maxStep = $state(1);
  let cond = $state<ConditionType>(initialCond);
  let name = $state(seed?.name ?? "");
  let prompt = $state(seed ? seed.prompt : STARTERS[initialCond]);
  let dirty = $state(seed ? seed.prompt !== STARTERS[initialCond] : false);
  let appBundleId = $state(
    seed?.condition.type === "app_opened" ? seed.condition.bundleId : "",
  );
  let appName = $state(seed?.condition.type === "app_opened" ? seed.condition.appName : "");
  let time = $state(seed?.condition.type === "schedule" ? seed.condition.time : "18:00");
  // "daily" or a single weekday — the backend's cadence model (daily | weekly).
  let schedDay = $state<"daily" | ScheduleWeekday>(
    seed?.condition.type === "schedule" && seed.condition.cadence === "weekly"
      ? (seed.condition.weekday ?? "monday")
      : "daily",
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
  let shareFlash = $state(false);
  let nameInput = $state<HTMLInputElement | null>(null);

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

  // ── Step 1: condition ─────────────────────────────────────────────────────
  function selectCond(next: ConditionType): void {
    cond = next;
    // Switching condition swaps the starter only while the prompt is unedited.
    if (!dirty) prompt = STARTERS[next];
  }

  const DAY_CHIPS: ReadonlyArray<{ key: "daily" | ScheduleWeekday; label: string }> = [
    { key: "daily", label: "every day" },
    { key: "monday", label: "Mon" },
    { key: "tuesday", label: "Tue" },
    { key: "wednesday", label: "Wed" },
    { key: "thursday", label: "Thu" },
    { key: "friday", label: "Fri" },
    { key: "saturday", label: "Sat" },
    { key: "sunday", label: "Sun" },
  ];
  const schedPreview = $derived(
    `Runs ${schedDay === "daily" ? "every day" : dayFull(schedDay)} at ${fmtTime(time || "18:00")}.`,
  );
  function dayFull(day: ScheduleWeekday): string {
    return `${day.charAt(0).toUpperCase()}${day.slice(1)}s`;
  }

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
      sub: `runs ${schedDay === "daily" ? "every day" : dayFull(schedDay)} at ${fmtTime(time || "18:00")}`,
    };
  });

  const ADV_ROWS = $derived.by(() =>
    [
      {
        key: "minlen" as const,
        label: "Minimum meeting length",
        min: 1,
        max: 30,
        step: 1,
        visible: cond === "meeting_ends",
      },
      {
        key: "awaygap" as const,
        label: "Away gap",
        min: 5,
        max: 120,
        step: 5,
        visible: cond === "app_opened",
      },
      { key: "cooldown" as const, label: "Cooldown", min: 0, max: 120, step: 5, visible: true },
    ].filter((row) => row.visible),
  );
  const allDefaults = $derived(
    adv.minlen === DEFAULT_MIN_MEETING_MINUTES &&
      adv.awaygap === DEFAULT_AWAY_GAP_MINUTES &&
      adv.cooldown === DEFAULT_COOLDOWN_MINUTES,
  );
  function bumpAdv(key: "minlen" | "awaygap" | "cooldown", delta: number, min: number, max: number): void {
    adv[key] = Math.min(max, Math.max(min, adv[key] + delta));
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
    return schedDay === "daily"
      ? { type: "schedule", cadence: "daily", time: time || "18:00" }
      : { type: "schedule", cadence: "weekly", time: time || "18:00", weekday: schedDay };
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
      shareFlash = true;
      setTimeout(() => (shareFlash = false), 1100);
    } catch {
      // Clipboard unavailable — quietly do nothing.
    }
  }

  function onNext(): void {
    if (step < 3) {
      goStep(step + 1);
      return;
    }
    void save();
  }
</script>

<div class="wiz-panel">
  <nav class="wiz-crumb">
    <button class="back" type="button" onclick={oncancel}>triggers</button>
    <span class="sep">/</span>
    <span class="here">{crumb}</span>
  </nav>

  {#if mode === "import"}
    <div class="warn-banner" role="status">
      <span class="glyph" aria-hidden="true">▲</span>
      <span>
        Imported — review this prompt before saving. It will run automatically, with read-only
        access to your capture history, whenever the condition fires.
      </span>
    </div>
  {/if}

  {#if createBlocked}
    <div class="warn-banner" role="status">
      <span class="glyph" aria-hidden="true">▲</span>
      <span>
        Triggers need an AI provider before they can be created — nothing can run without one.
        <button type="button" class="banner-link" onclick={onsetupprovider}>Set up provider</button>
      </span>
    </div>
  {/if}

  <div class="steps">
    {#each [{ n: 1, label: "Condition" }, { n: 2, label: "Prompt" }, { n: 3, label: "Review" }] as tab, index (tab.n)}
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
        <span class="num">0{tab.n}</span>{tab.label}
      </button>
    {/each}
  </div>

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
      <div class="cond-pick" role="radiogroup" aria-label="Condition">
        {#each CONDITION_SECTIONS as card (card.cond)}
          <div
            class="cond-card"
            class:selected={cond === card.cond}
            role="radio"
            tabindex="0"
            aria-checked={cond === card.cond}
            onclick={() => selectCond(card.cond)}
            onkeydown={(event) => {
              if (event.key === " " || event.key === "Enter") {
                event.preventDefault();
                selectCond(card.cond);
              }
            }}
          >
            <span class="radio" aria-hidden="true"></span>
            {#if card.cond === "meeting_ends"}
              <span class="cond-name">When a meeting ends</span>
              <span class="cond-desc">
                A conferencing app (Zoom, Teams, Meet in a browser…) held your mic for at least 5
                minutes, then released it and stayed quiet for ~2 minutes.
              </span>
            {:else if card.cond === "app_opened"}
              <span class="cond-name">When an app opens</span>
              <span class="cond-desc">
                A chosen app comes to the front after you've been away from it for 30 minutes or
                more — a fresh working session, not window switching.
              </span>
            {:else}
              <span class="cond-name">On a schedule</span>
              <span class="cond-desc">
                At a fixed local time, daily or on a weekday you pick — good for daily wrap-ups and
                weekly reviews.
              </span>
            {/if}
          </div>
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
                    <div class="day-chips" role="radiogroup" aria-label="Days">
                      {#each DAY_CHIPS as chip (chip.key)}
                        <button
                          type="button"
                          class="day-chip"
                          class:on={schedDay === chip.key}
                          role="radio"
                          aria-checked={schedDay === chip.key}
                          onclick={() => (schedDay = chip.key)}
                        >{chip.label}</button>
                      {/each}
                    </div>
                  </div>
                </div>
                <p class="sched-preview">{schedPreview}</p>
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
      <p class="wiz-lead">
        <span class="q">{mode === "edit" ? "Review and save." : "Review and create."}</span>
        <span class="hint">
          Name it, check the pieces, tune Advanced only if you must — the defaults are right for
          almost everyone.
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
            <li><span class="tick">✓</span> the firing window — condition, time span, app</li>
            <li><span class="tick">✓</span> your User Context — what Mnema has learned about your work</li>
            <li><span class="tick">✓</span> speaker identity — which voice in the audio is you</li>
            <li><span class="tick">✓</span> previous runs of this trigger, so results compound</li>
          </ul>
          <p class="auto-note">
            Assembled on every run. Nothing to configure — want a generic result? Just say so in
            the prompt.
          </p>
        </div>

        <div class="review-sec">
          <details class="disclosure" bind:open={advOpen}>
            <summary>
              <span class="caret" aria-hidden="true">▸</span> Advanced
              <span class="default-note">{allDefaults ? "all defaults" : "modified"}</span>
            </summary>
            <div class="disc-body">
              {#each ADV_ROWS as row (row.key)}
                <div class="stepper-row">
                  <span class="stepper-lbl">{row.label}</span>
                  <span class="stepper">
                    <button
                      type="button"
                      aria-label={`Decrease ${row.label}`}
                      onclick={() => bumpAdv(row.key, -row.step, row.min, row.max)}
                    >−</button>
                    <span class="val">{adv[row.key]} min</span>
                    <button
                      type="button"
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

  <div class="wiz-nav">
    {#if step > 1}
      <button class="btn" type="button" onclick={() => goStep(step - 1)}>◀ Back</button>
    {/if}
    <span class="spacer"></span>
    {#if step === 3}
      <span class="note">saved to triggers.json — runs stay on this Mac</span>
      <button
        class="btn btn--ghost"
        type="button"
        title="Copies Trigger JSON — never carries provider or model config"
        onclick={() => void shareJson()}
      >{shareFlash ? "copied ✓" : "Share as JSON"}</button>
    {/if}
    <button
      class="btn btn--accent"
      type="button"
      disabled={saving || (step === 3 && createBlocked)}
      onclick={onNext}
    >
      {step < 3 ? "Next ▶" : mode === "edit" ? "Save Changes" : "Create Trigger"}
    </button>
  </div>
</div>
