<script lang="ts">
  // Meetings surface — the day-grouped list of detected meetings with a
  // drill-in detail (Warm Paper redesign, Slice 5; anatomy per
  // docs/mockups/unified-shell/app-match/meetings.html frame 1). Renders
  // inside the shared InsightsShell main column; the owning route passes the
  // chat-handoff callback used to open a recap's run conversation.
  import { untrack } from "svelte";
  import { listMeetings, type MeetingDay, type MeetingSummary } from "./api";
  import MeetingDetail from "./MeetingDetail.svelte";
  import {
    appGlyph,
    dayHeading,
    dayTotals,
    durationLabel,
    isUnknownSpeaker,
    meetingTitle,
    provenanceLabel,
    timeRange,
  } from "./format";

  interface Props {
    onOpenConversation: (conversationId: string) => void;
  }

  let { onOpenConversation }: Props = $props();

  // null = still loading (distinct from loaded-and-empty).
  let days = $state<MeetingDay[] | null>(null);
  let loadError = $state<string | null>(null);
  let selected = $state<MeetingSummary | null>(null);

  $effect(() => {
    void untrack(async () => {
      try {
        days = await listMeetings(-new Date().getTimezoneOffset());
      } catch (e) {
        loadError = String(e);
      }
    });
  });

  const totalCount = $derived(
    days?.reduce((n, d) => n + d.meetings.length, 0) ?? 0,
  );

  function stateLabel(state: MeetingSummary["state"]): string {
    switch (state) {
      case "recap":
        return "recap ready";
      case "processing":
        return "transcribing · recap after";
      case "skipped":
        return "skipped";
      default:
        return "transcript only";
    }
  }

  /** Chip shown in place of speakers when diarization has nothing (yet). */
  function noVoicesLabel(m: MeetingSummary): string {
    return m.state === "processing"
      ? "voices pending"
      : m.state === "skipped"
        ? "not recorded"
        : "no voices matched";
  }
</script>

{#if selected !== null}
  <MeetingDetail
    meeting={selected}
    onBack={() => (selected = null)}
    {onOpenConversation}
  />
{:else}
  <div class="meetings">
    <div class="page-head">
      <h1 class="page-title">Meetings</h1>
      <p class="page-sub">
        Detected when a conferencing app holds your microphone for five minutes
        or more — no calendar connected, none needed.
        {#if totalCount > 0}<span class="m"
            >{totalCount} recent</span
          >{/if}
      </p>
    </div>

    {#if loadError !== null}
      <p class="page-quiet">Couldn't load meetings. {loadError}</p>
    {:else if days === null}
      <!-- Loading: quiet — the list appears when it appears. -->
    {:else if totalCount === 0}
      <div class="empty">
        <p class="empty-eyebrow">
          <span class="g" aria-hidden="true">◉</span> Meetings
        </p>
        <h2 class="empty-title">No meetings yet today.</h2>
        <p class="empty-detail">
          When an app holds your microphone for five minutes or more, the
          meeting shows up here with its transcript — and a recap, if a Meeting
          Recap trigger is set up.
        </p>
      </div>
    {:else}
      <div class="list">
        {#each days as dayGroup (dayGroup.day)}
          {@const heading = dayHeading(dayGroup.day)}
          <div class="day-head">
            <h3>{heading.label}</h3>
            <span class="m">{heading.sub}</span>
            <span class="rule"></span>
            <span class="m">{dayTotals(dayGroup.meetings)}</span>
          </div>
          {#each dayGroup.meetings as meeting (meeting.id)}
            <button
              type="button"
              class="meet-row"
              class:muted={meeting.state === "skipped"}
              onclick={() => (selected = meeting)}
            >
              <div class="app-glyph" class:web={meeting.meetingUrl}>
                {appGlyph(meeting)}
              </div>
              <div class="meet-main">
                <div class="meet-title">{meetingTitle(meeting)}</div>
                <div class="meet-meta">
                  <span class="m">
                    {timeRange(meeting.startMs, meeting.endMs)} · {durationLabel(
                      meeting.endMs - meeting.startMs,
                    )}
                  </span>
                  <div class="chips">
                    {#if meeting.speakers.length === 0}
                      <span class="spk unk">{noVoicesLabel(meeting)}</span>
                    {:else}
                      {#each meeting.speakers as speaker (speaker)}
                        <span
                          class="spk"
                          class:you={speaker === "You"}
                          class:unk={isUnknownSpeaker(speaker)}>{speaker}</span
                        >
                      {/each}
                    {/if}
                  </div>
                </div>
                <div class="meet-prov">
                  <span class="g" aria-hidden="true">◉</span>
                  {provenanceLabel(meeting) +
                    (meeting.state === "skipped" && meeting.reason
                      ? ` · ${meeting.reason}`
                      : "")}
                </div>
              </div>
              <span class="state {meeting.state}">
                {#if meeting.state === "processing"}<span
                    class="spin"
                    aria-hidden="true"
                  ></span>{/if}{stateLabel(meeting.state)}
              </span>
            </button>
          {/each}
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .meetings {
    --serif: var(--app-font-narrative);
    max-width: 880px;
    margin: 0 auto;
    padding: 32px 12px 40px;
  }

  .page-title {
    font-family: var(--serif);
    font-size: 32px;
    font-weight: 400;
    letter-spacing: -0.01em;
    margin: 0;
    color: var(--app-text-strong);
  }
  .page-sub {
    font-size: 14px;
    color: var(--app-text-muted);
    margin: 8px 0 0;
    max-width: 720px;
    line-height: 1.5;
  }
  .page-sub .m {
    font-family: var(--app-font-mono);
    font-size: 12px;
    color: var(--app-text-faint);
  }
  .page-quiet {
    margin: 26px 0 0;
    font-size: var(--text-md);
    color: var(--app-text-muted);
  }

  .list {
    margin-top: 28px;
  }

  .day-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin: 26px 0 12px;
  }
  .day-head:first-child {
    margin-top: 0;
  }
  .day-head h3 {
    font-family: var(--serif);
    font-size: 17px;
    font-weight: 500;
    margin: 0;
    color: var(--app-text-strong);
  }
  .day-head .m {
    font-family: var(--app-font-mono);
    font-size: 11.5px;
    color: var(--app-text-faint);
  }
  .day-head .rule {
    flex: 1;
    height: 1px;
    background: var(--app-border);
  }

  .meet-row {
    display: flex;
    align-items: center;
    gap: 18px;
    width: 100%;
    text-align: left;
    font: inherit;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 12px;
    padding: 15px 20px;
    margin-bottom: 10px;
    cursor: pointer;
    transition:
      border-color 0.13s ease,
      box-shadow 0.13s ease,
      transform 0.13s ease;
  }
  .meet-row:hover {
    border-color: var(--app-accent-border);
    transform: translateY(-1px);
  }
  .meet-row:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .meet-row.muted {
    background: var(--app-surface-subtle);
  }
  .meet-row.muted .meet-title {
    color: var(--app-text-muted);
  }

  .app-glyph {
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: 10px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--app-font-mono);
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text-muted);
  }
  .app-glyph.web {
    font-size: 16px;
  }

  .meet-main {
    flex: 1;
    min-width: 0;
  }
  .meet-title {
    font-family: var(--serif);
    font-size: 17px;
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meet-meta {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 5px;
    flex-wrap: wrap;
  }
  .meet-meta .m {
    font-family: var(--app-font-mono);
    font-size: 11.5px;
    color: var(--app-text-faint);
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

  .meet-prov {
    font-family: var(--app-font-mono);
    font-size: 10.5px;
    color: var(--app-text-faint);
    margin-top: 6px;
    letter-spacing: 0.02em;
  }
  .meet-prov .g {
    color: var(--app-accent);
  }

  /* ── State pills ── */
  .state {
    flex: none;
    font-family: var(--app-font-mono);
    font-size: 11px;
    border-radius: 999px;
    padding: 5px 12px;
    letter-spacing: 0.02em;
  }
  .state.recap {
    color: var(--app-accent-strong);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
  }
  .state.transcript_only {
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
  }
  .state.processing {
    color: var(--app-warn-strong);
    background: var(--app-warn-bg);
    border: 1px solid var(--app-warn-border);
  }
  .state.skipped {
    color: var(--app-text-faint);
    background: transparent;
    border: 1px dashed var(--app-border-strong);
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

  /* ── Empty state — calm, explains detection in one sentence ── */
  .empty {
    margin: 96px auto 0;
    max-width: 420px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    text-align: center;
    align-items: center;
  }
  .empty-eyebrow {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: var(--text-xs);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .empty-eyebrow .g {
    color: var(--app-accent);
    letter-spacing: 0;
  }
  .empty-title {
    margin: 0;
    font-family: var(--serif);
    font-size: 22px;
    font-weight: 400;
    line-height: 1.35;
    color: var(--app-text-strong);
  }
  .empty-detail {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.6;
    color: var(--app-text-muted);
  }
</style>
