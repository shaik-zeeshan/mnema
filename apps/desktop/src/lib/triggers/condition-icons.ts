// Condition iconography — the single source of truth for the lucide icon that
// identifies each condition type everywhere it appears (list section headers,
// wizard condition cards, runs ledger, document-view eyebrow, rail origin
// badge). Replaces the DESIGN.md unicode glyphs (◉ ▣ ◷): the app's icon pack
// is lucide via unplugin-icons (the `section-icons.ts` pattern).
//
// Kept separate from `./api` so the pure share/import logic there stays
// importable under plain `bun test` (no `~icons` virtual modules).
import type { Component } from "svelte";
import type { SvelteHTMLElements } from "svelte/elements";

import IconAudioLines from "~icons/lucide/audio-lines";
import IconAppWindow from "~icons/lucide/app-window";
import IconClock from "~icons/lucide/clock";

import type { ConditionType } from "./api";

/** A lucide icon as produced by unplugin-icons (`~icons/lucide/*`). */
export type ConditionIconComponent = Component<SvelteHTMLElements["svg"]>;

export const CONDITION_ICON: Record<ConditionType, ConditionIconComponent> = {
  meeting_ends: IconAudioLines,
  app_opened: IconAppWindow,
  schedule: IconClock,
};
