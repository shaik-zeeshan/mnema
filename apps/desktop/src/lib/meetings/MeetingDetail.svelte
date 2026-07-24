<script lang="ts">
  // Meeting detail — Summary / Transcript / Notes tabs over one detected
  // meeting (Warm Paper redesign, Slice 5; anatomy per
  // docs/mockups/unified-shell/app-match/meetings.html frame 2).
  //
  // Summary: the recap lives in its trigger-run CONVERSATION — this pane
  // states that honestly per state and opens the conversation via the shared
  // chat handoff (no document re-render here; Chat owns that).
  // Transcript: speaker-labeled turns with wall-clock mono times.
  // Notes: one editable text field persisted via set_meeting_notes
  // (debounced + save-on-blur; flushed on unmount).
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    getMeeting,
    setMeetingChecklist,
    setMeetingNotes,
    type MeetingDetail,
    type MeetingSummary,
  } from "./api";
  import {
    durationLabel,
    isUnknownSpeaker,
    meetingTitle,
    provenanceLabel,
    timeRange,
  } from "./format";
  import { renderMarkdown } from "$lib/markdown";
  import { triggerDocTurnIndex, type Conversation } from "$lib/insights/conversation";

  interface Props {
    meeting: MeetingSummary;
    onBack: () => void;
    onOpenConversation: (conversationId: string) => void;
  }

  let { meeting, onBack, onOpenConversation }: Props = $props();

  let detail = $state<MeetingDetail | null>(null);
  let loadError = $state<string | null>(null);
  let tab = $state<"summary" | "transcript" | "notes">("summary");

  // ── Notes: debounced autosave + save-on-blur ─────────────────────────────
  let notes = $state("");
  let savedNotes = "";
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let saveFailed = $state(false);

  async function saveNotes(): Promise<void> {
    clearTimeout(saveTimer);
    const value = notes;
    if (value === savedNotes) return;
    try {
      await setMeetingNotes(meeting.id, value.trim() === "" ? null : value);
      savedNotes = value;
      saveFailed = false;
    } catch {
      saveFailed = true;
    }
  }

  function onNotesInput(): void {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void saveNotes(), 800);
  }

  $effect(() => {
    const id = meeting.id;
    untrack(() => {
      detail = null;
      loadError = null;
      void getMeeting(id).then(
        (d) => {
          detail = d;
          notes = d.notes ?? "";
          savedNotes = notes;
        },
        (e) => {
          loadError = String(e);
        },
      );
    });
    // Flush a pending debounce when the detail unmounts / switches meeting.
    return () => void saveNotes();
  });

  // ── Summary: the recap, rendered inline (no chat handoff) ────────────────
  // The recap lives in its trigger-run conversation; fetch it, pick the
  // Document-View turn (ADR 0058), and render its markdown here. Task-list
  // action items become checkboxes whose ticked-state persists per meeting.
  let recapHtml = $state<string | null>(null); // null = loading; "" = empty recap
  let recapFailed = $state(false);
  let recapEl = $state<HTMLElement | null>(null);
  let recapToken = 0; // drops a stale fetch when the meeting switches

  $effect(() => {
    const convoId =
      meeting.state === "recap" ? (meeting.conversationId ?? null) : null;
    recapHtml = null;
    recapFailed = false;
    const token = ++recapToken;
    if (convoId === null) return;
    untrack(() => {
      void invoke<Conversation | null>("get_conversation", {
        conversationId: convoId,
      }).then(
        (convo) => {
          if (token !== recapToken) return;
          const idx = convo ? triggerDocTurnIndex(convo.turns) : -1;
          const answer = idx >= 0 ? convo!.turns[idx].answer : "";
          recapHtml = answer.trim().length > 0 ? renderMarkdown(answer) : "";
        },
        () => {
          if (token === recapToken) recapFailed = true;
        },
      );
    });
  });

  // Reflect the saved ticked-state onto the rendered checkboxes (keyed by item
  // text — stable across re-runs, unlike an index). Runs after {@html} paints.
  $effect(() => {
    recapHtml;
    const el = recapEl;
    if (el === null || recapHtml === null || recapHtml === "") return;
    const ticked = new Set(detail?.checklist ?? []);
    for (const item of el.querySelectorAll<HTMLElement>("li.task-item")) {
      const box = item.querySelector<HTMLInputElement>(".task-checkbox");
      if (box !== null) box.checked = ticked.has(item.textContent?.trim() ?? "");
    }
  });

  // A checkbox toggled: persist every ticked item's text (change bubbles up).
  function onRecapChange(event: Event): void {
    const target = event.target as HTMLElement | null;
    if (target === null || !target.classList.contains("task-checkbox")) return;
    const el = recapEl;
    if (el === null) return;
    const ticked: string[] = [];
    for (const item of el.querySelectorAll<HTMLElement>("li.task-item")) {
      const box = item.querySelector<HTMLInputElement>(".task-checkbox");
      const text = item.textContent?.trim();
      if (box?.checked && text) ticked.push(text);
    }
    void setMeetingChecklist(meeting.id, ticked);
  }

  const startDate = $derived(new Date(meeting.startMs));
  const dateLabel = $derived(
    `${startDate.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric", year: "numeric" }).toUpperCase().replaceAll(",", "")}`,
  );
  const turnCount = $derived(detail?.turns.length ?? null);

  // Stable per-speaker color assignment: "You" is always ink-strong; other
  // voices cycle a small token palette in first-seen order.
  const speakerClass = $derived.by(() => {
    const map = new Map<string, string>();
    let next = 0;
    for (const t of detail?.turns ?? []) {
      if (t.speaker === "You") map.set(t.speaker, "you");
      else if (!map.has(t.speaker)) map.set(t.speaker, `s${(next++ % 4) + 1}`);
    }
    return (speaker: string) => map.get(speaker) ?? "s1";
  });

  const turnTimeFmt = new Intl.DateTimeFormat("en-US", {
    hour: "numeric",
    minute: "2-digit",
    hour12: false,
  });
</script>

<div class="detail" class:detail--wide={tab === "transcript"}>
  <button type="button" class="back" onclick={onBack}>← MEETINGS</button>
  <h1 class="title">{meetingTitle(meeting)}</h1>
  <div class="meta">
    <span class="m">
      {dateLabel} · {timeRange(meeting.startMs, meeting.endMs)} · {durationLabel(
        meeting.endMs - meeting.startMs,
      )}
    </span>
    {#if meeting.speakers.length > 0}
      <span class="sep">·</span>
      <div class="chips">
        {#each meeting.speakers as speaker (speaker)}
          <span
            class="spk"
            class:you={speaker === "You"}
            class:unk={isUnknownSpeaker(speaker)}>{speaker}</span
          >
        {/each}
      </div>
    {/if}
  </div>
  <p class="provenance">
    <span class="g" aria-hidden="true">◉</span>
    {provenanceLabel(meeting) +
      (meeting.speakers.length > 0
        ? ` · ${meeting.speakers.length} voice${meeting.speakers.length === 1 ? "" : "s"}`
        : "")}
  </p>

  <div class="tabs" role="tablist" aria-label="Meeting detail">
    <button
      type="button"
      class="tab"
      class:active={tab === "summary"}
      role="tab"
      aria-selected={tab === "summary"}
      onclick={() => (tab = "summary")}>Summary</button
    >
    <button
      type="button"
      class="tab"
      class:active={tab === "transcript"}
      role="tab"
      aria-selected={tab === "transcript"}
      onclick={() => (tab = "transcript")}
      >Transcript{#if turnCount !== null}<span class="n">{turnCount} turns</span
        >{/if}</button
    >
    <button
      type="button"
      class="tab"
      class:active={tab === "notes"}
      role="tab"
      aria-selected={tab === "notes"}
      onclick={() => (tab = "notes")}>Notes</button
    >
  </div>

  {#if loadError !== null}
    <p class="quiet">Couldn't load this meeting. {loadError}</p>
  {:else if tab === "summary"}
    {#if meeting.state === "recap" && meeting.conversationId}
      <div class="run-eyebrow">
        <span class="g" aria-hidden="true">◉</span> trigger run · meeting recap
      </div>
      {#if recapFailed}
        <p class="quiet">
          Couldn't load the recap.
          <button
            type="button"
            class="linklike"
            onclick={() => onOpenConversation(meeting.conversationId!)}
            >Open it in chat</button
          >.
        </p>
      {:else if recapHtml === null}
        <p class="quiet"><span class="spin" aria-hidden="true"></span> Loading recap…</p>
      {:else if recapHtml === ""}
        <p class="quiet">
          The recap ran but produced no text.
          <button
            type="button"
            class="linklike"
            onclick={() => onOpenConversation(meeting.conversationId!)}
            >Open the conversation</button
          >.
        </p>
      {:else}
        <!-- Rendered recap markdown; task-list items are persisted checkboxes.
             {@html} is safe: renderMarkdown runs with html:false (source HTML
             is escaped; the only injected markup is its own checkbox literal). -->
        <div class="recap-doc" bind:this={recapEl} onchange={onRecapChange}>
          {@html recapHtml}
        </div>
        <div class="recap-foot">
          <button
            type="button"
            class="open-convo"
            onclick={() => onOpenConversation(meeting.conversationId!)}
          >
            Open full conversation
            <svg
              width="13"
              height="13"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"><path d="M4 12 12 4M6 4h6v6" /></svg
            >
          </button>
        </div>
      {/if}
    {:else if meeting.state === "processing"}
      <p class="quiet">
        <span class="spin" aria-hidden="true"></span>
        Transcribing… the recap runs once the transcript is ready.
      </p>
    {:else if meeting.state === "skipped"}
      <p class="quiet">
        Recap skipped — {meeting.reason ??
          "nothing was recorded for this meeting"}.
      </p>
    {:else}
      <p class="quiet">
        No recap trigger ran for this meeting. The transcript is on the
        Transcript tab.
      </p>
    {/if}
  {:else if tab === "transcript"}
    {#if detail === null}
      <p class="quiet">Loading transcript…</p>
    {:else if detail.turns.length === 0}
      <p class="quiet">No transcript for this meeting.</p>
    {:else}
      <p class="turns-note">
        voices separated by diarization, matched to saved speakers · times are
        wall-clock
      </p>
      <div class="turns">
        {#each detail.turns as turn, i (i)}
          <div class="turn">
            <span class="ts">{turnTimeFmt.format(new Date(turn.startedAtMs))}</span>
            <span class="who {speakerClass(turn.speaker)}">{turn.speaker}</span>
            <span class="say">{turn.text}</span>
          </div>
        {/each}
      </div>
    {/if}
  {:else}
    <div class="notes-card">
      <textarea
        class="notes-input"
        placeholder="Write anything worth keeping about this meeting…"
        bind:value={notes}
        oninput={onNotesInput}
        onblur={() => void saveNotes()}
        disabled={detail === null}
      ></textarea>
      <p class="hint">
        {#if saveFailed}
          couldn't save — edits retry on your next change
        {:else}
          your notes stay local · never sent to any model unless you paste them
          into a chat
        {/if}
      </p>
    </div>
  {/if}
</div>

<style>
  .detail {
    --serif: var(--app-font-narrative);
    max-width: 780px;
    margin: 0 auto;
    padding: 32px 20px 60px;
    transition: max-width 0.16s ease;
  }
  /* The transcript is a two-column dialogue (speaker + text), not prose — give
     it more room than the 780px reading measure so lines don't wrap early. */
  .detail--wide {
    max-width: 1040px;
  }

  .back {
    font-family: var(--app-font-mono);
    font-size: 11.5px;
    letter-spacing: 0.04em;
    color: var(--app-text-faint);
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .back:hover {
    color: var(--app-accent);
  }
  .back:focus-visible {
    outline: none;
    border-radius: 4px;
    box-shadow: var(--app-ring);
  }

  .title {
    font-family: var(--serif);
    font-size: 34px;
    font-weight: 400;
    letter-spacing: -0.012em;
    line-height: 1.2;
    margin: 18px 0 0;
    color: var(--app-text-strong);
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
    margin-top: 14px;
  }
  .meta .m {
    font-family: var(--app-font-mono);
    font-size: 12px;
    color: var(--app-text-muted);
  }
  .meta .sep {
    color: var(--app-border-strong);
  }

  .chips {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .spk {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 999px;
    padding: 2.5px 9px;
  }
  .spk.you {
    color: var(--app-accent-strong);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  .spk.unk {
    color: var(--app-text-faint);
    border-style: dashed;
  }

  .provenance {
    font-family: var(--app-font-mono);
    font-size: 11px;
    color: var(--app-text-faint);
    letter-spacing: 0.02em;
    margin: 12px 0 0;
  }
  .provenance .g {
    color: var(--app-accent);
  }

  /* ── Tabs ── */
  .tabs {
    display: flex;
    gap: 4px;
    margin-top: 30px;
    border-bottom: 1px solid var(--app-border);
  }
  .tab {
    font: inherit;
    font-size: 13.5px;
    font-weight: 500;
    color: var(--app-text-muted);
    background: none;
    border: 0;
    cursor: pointer;
    padding: 9px 16px 11px;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    transition:
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .tab:hover {
    color: var(--app-text-strong);
  }
  .tab:focus-visible {
    outline: none;
    color: var(--app-text-strong);
    box-shadow: var(--app-ring);
    border-radius: 6px 6px 0 0;
  }
  .tab.active {
    color: var(--app-accent-strong);
    border-bottom-color: var(--app-accent);
  }
  .tab .n {
    font-family: var(--app-font-mono);
    font-size: 10.5px;
    color: var(--app-text-faint);
    margin-left: 5px;
  }

  .quiet {
    margin: 26px 0 0;
    font-size: var(--text-md);
    line-height: 1.6;
    color: var(--app-text-muted);
  }

  /* ── Summary (recap) ── */
  .run-eyebrow {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 26px;
    font-family: var(--app-font-mono);
    font-size: 10.5px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .run-eyebrow .g {
    color: var(--app-accent);
  }
  .linklike {
    font: inherit;
    color: var(--app-accent-strong);
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .linklike:hover {
    text-decoration: underline;
  }

  /* Rendered recap markdown. Mirrors the AnswerProse prose + task-list styling
     (markdown.ts's `task-item`/`task-checkbox` contract) so the inline recap
     reads like the chat document it came from. */
  .recap-doc {
    margin-top: 16px;
    font-size: 14.5px;
    line-height: 1.65;
    color: var(--app-text);
  }
  .recap-doc :global(p),
  .recap-doc :global(ul),
  .recap-doc :global(ol) {
    margin: 0 0 0.8em;
  }
  .recap-doc :global(ul),
  .recap-doc :global(ol) {
    padding-left: 1.4em;
  }
  .recap-doc :global(li) {
    margin: 0.2em 0;
  }
  .recap-doc :global(h1),
  .recap-doc :global(h2),
  .recap-doc :global(h3) {
    font-family: var(--serif);
    font-weight: 500;
    line-height: 1.3;
    margin: 1.1em 0 0.4em;
    color: var(--app-text-strong);
  }
  .recap-doc :global(h1) {
    font-size: 1.35em;
  }
  .recap-doc :global(h2) {
    font-size: 1.2em;
  }
  .recap-doc :global(h3) {
    font-size: 1.08em;
  }
  .recap-doc :global(strong) {
    color: var(--app-text-strong);
    font-weight: 650;
  }
  .recap-doc :global(a) {
    color: var(--app-accent-strong);
  }
  .recap-doc :global(code) {
    font-family: var(--app-font-mono);
    font-size: 0.88em;
    padding: 0.1em 0.35em;
    border-radius: 4px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
  }
  .recap-doc :global(li.task-item) {
    list-style: none;
    margin-left: -1.2em;
  }
  .recap-doc :global(.task-checkbox) {
    margin: 0 0.5em 0 0;
    vertical-align: -0.15em;
    accent-color: var(--app-accent);
    cursor: pointer;
  }
  .recap-doc :global(li.task-item:has(.task-checkbox:checked)) {
    color: var(--app-text-muted);
    text-decoration: line-through;
    text-decoration-color: var(--app-border-strong);
  }
  .recap-foot {
    margin-top: 22px;
    padding-top: 20px;
    border-top: 1px solid var(--app-border);
  }
  .open-convo {
    font: inherit;
    font-size: 13.5px;
    font-weight: 500;
    color: var(--app-accent-strong);
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border: 1px solid var(--app-accent-border);
    border-radius: 8px;
    background: var(--app-accent-bg);
    cursor: pointer;
    white-space: nowrap;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .open-convo:hover {
    border-color: var(--app-accent);
  }
  .open-convo:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  .spin {
    display: inline-block;
    width: 9px;
    height: 9px;
    margin-right: 6px;
    border-radius: 50%;
    border: 1.5px solid var(--app-warn-border);
    border-top-color: var(--app-warn-strong);
    vertical-align: -1px;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* ── Transcript ── */
  .turns-note {
    font-family: var(--app-font-mono);
    font-size: 10.5px;
    color: var(--app-text-faint);
    letter-spacing: 0.02em;
    margin: 22px 0 4px;
  }
  .turns {
    margin-top: 8px;
  }
  .turn {
    display: flex;
    gap: 18px;
    padding: 13px 6px;
    border-bottom: 1px solid var(--app-border);
  }
  .turn:hover {
    background: var(--app-surface-hover);
  }
  .turn .ts {
    font-family: var(--app-font-mono);
    font-size: 11px;
    color: var(--app-text-faint);
    flex: none;
    width: 52px;
    padding-top: 3px;
    font-variant-numeric: tabular-nums;
  }
  .turn .who {
    font-family: var(--app-font-mono);
    font-size: 11.5px;
    font-weight: 700;
    flex: none;
    /* Show the full given name (profile name / label); wrap rather than clip so
       "Sarah Chen" or "Meeting audio" is never truncated. */
    width: 136px;
    padding-top: 2px;
    overflow-wrap: break-word;
  }
  .turn .who.you {
    color: var(--app-text-strong);
  }
  .turn .who.s1 {
    color: var(--app-accent-strong);
  }
  .turn .who.s2 {
    color: var(--app-warn-strong);
  }
  .turn .who.s3 {
    color: var(--app-info-strong);
  }
  .turn .who.s4 {
    color: var(--app-source-mic-strong);
  }
  .turn .say {
    font-size: 14px;
    line-height: 1.6;
    color: var(--app-text);
    min-width: 0;
  }

  /* ── Notes ── */
  .notes-card {
    margin-top: 26px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 12px;
    padding: 20px 22px;
  }
  .notes-input {
    display: block;
    width: 100%;
    min-height: 280px;
    resize: vertical;
    font: inherit;
    font-size: 14.5px;
    line-height: 1.7;
    color: var(--app-text);
    background: transparent;
    border: 0;
    padding: 0;
  }
  .notes-input::placeholder {
    color: var(--app-text-faint);
  }
  .notes-input:focus {
    outline: none;
  }
  .notes-card:focus-within {
    border-color: var(--app-border-hover);
  }
  .notes-card .hint {
    font-family: var(--app-font-mono);
    font-size: 11px;
    color: var(--app-text-faint);
    margin: 14px 0 0;
  }
</style>
