<script lang="ts">
  // Conversations (2×1, 1×1 at the 800px floor). Round-4 rule: audio is counted
  // as CONVERSATIONS, never minutes — the header meta is a count, and a row's
  // length is a detail under its title rather than the headline.
  //
  // Data: `get_conversations` (Activities whose window overlaps ≥2 min of
  // recorded speech). A row opens the Timeline at that conversation by resolving
  // one real frame inside its window (`get_latest_frame_in_range`) and handing
  // it to the existing `open_capture_result_in_main_window` seam.
  import { invoke } from "@tauri-apps/api/core";
  import type { ConversationCluster } from "$lib/highlights";
  import type { FrameDto } from "$lib/types/app-infra";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { clock, minutesLabel } from "./overview-format";
  import Glyph from "./Glyph.svelte";

  let {
    conversations,
    loaded,
  }: { conversations: ConversationCluster[]; loaded: boolean } = $props();

  const rows = $derived(conversations.slice(0, 2));

  // Audio on at all? Decides which of the two empty-state second lines is true.
  const audioOn = $derived.by(() => {
    const s = captureControls.recordingSettings;
    return s ? s.captureMicrophone || s.captureSystemAudio : null;
  });

  async function open(row: ConversationCluster): Promise<void> {
    try {
      const frame = await invoke<FrameDto | null>("get_latest_frame_in_range", {
        request: {
          capturedAtStart: new Date(row.startedAtMs).toISOString(),
          capturedAtEnd: new Date(row.endedAtMs).toISOString(),
        },
      });
      if (!frame) return;
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: frame.id,
        audioSegmentId: null,
        spanStartMs: null,
        alignedFrameId: null,
      });
    } catch {
      // Best-effort hand-off; a failed resolve leaves the Overview in place.
    }
  }
</script>

<div class="tile narrow-1">
  <div class="tile__h">
    <span class="t-label">Conversations</span>
    {#if conversations.length}
      <span class="tile__more is-num">{conversations.length} today</span>
    {/if}
  </div>

  {#if rows.length}
    <div class="pay pay--rows">
      {#each rows as row (row.activityId)}
        <button type="button" class="row" onclick={() => void open(row)}>
          <span class="row__txt">
            <span class="row__lbl">{row.title}</span>
            <span class="row__sub">
              {minutesLabel(row.spokenMs)} · {row.speakerCount}
              {row.speakerCount === 1 ? "speaker" : "speakers"}
            </span>
          </span>
          <span class="row__val">
            <span class="t-meta is-mono is-num">{clock(row.startedAtMs)}</span>
            <span class="chev"><Glyph name="chevr" /></span>
          </span>
        </button>
      {/each}
    </div>
  {:else}
    <div class="pay empty">
      <span class="empty__g"><Glyph name="sys" /></span>
      <span class="t-meta">{loaded ? "No speech recorded today" : "Reading the day…"}</span>
      {#if loaded && audioOn !== null}
        <span class="t-meta faint">
          {audioOn
            ? "audio is on · nothing to show yet"
            : "microphone and system audio are off"}
        </span>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Two columns wide by default; the 800px floor demotes it to one cell — still
     one of the four legal footprints, just the smaller one. */
  .tile {
    grid-column: span 2;
  }
  @media (max-width: 900px) {
    .narrow-1 {
      grid-column: span 1;
    }
  }

  button.row {
    width: 100%;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  button.row:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-6);
    text-align: center;
  }
  .empty__g {
    width: 18px;
    height: 18px;
    color: var(--app-text-faint);
  }
  .faint {
    color: var(--app-text-subtle);
  }
</style>
