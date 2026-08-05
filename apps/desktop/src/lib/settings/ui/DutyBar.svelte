<script lang="ts">
  // The OCR duty-cycle bar (direction 04's `.duty`). BOTH halves are drawn,
  // because the cooldown IS the pacing — a bar showing only the work share
  // would hide the half that keeps the machine quiet.
  //
  // Read-only: the split is a shipped governor constant, not a setting (see
  // `state/ocr-pacing.ts`). G8 — no temperature claim and no ETA; the line
  // underneath states the split in seconds and, where the caller has one, the
  // real backlog. Nothing about heat, and no invented "done in N minutes".
  import type { DutyCycle } from "../state/ocr-pacing";

  interface Props {
    cycle: DutyCycle;
    /** Optional real backlog phrase from `backlogPhrase()`. */
    backlog?: string | null;
  }

  let { cycle, backlog = null }: Props = $props();
</script>

<div class="duty">
  <p class="duty__band">{cycle.label}</p>
  <div
    class="duty__bar"
    role="img"
    aria-label="{cycle.label}: {cycle.workPercent}% of each minute reading text, {cycle.coolPercent}% idle"
  >
    <span class="duty__work" style:width="{cycle.workPercent}%">{cycle.workPercent}%</span>
    <span class="duty__cool" style:width="{cycle.coolPercent}%">COOL {cycle.coolPercent}%</span>
  </div>
  <p class="duty__note">{backlog ? `${cycle.phrase} · ${backlog}` : cycle.phrase}</p>
</div>
