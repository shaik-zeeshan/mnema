// The shared settings/onboarding glyph family — one Lucide component per
// `IconName` (every `SettingsSectionId` plus the standalone `lock`). Replaces
// the old hand-drawn `Icon.svelte` switch; both the settings rail
// (`SettingsRail.svelte`) and the onboarding feature rows (`FeatureRow.svelte`)
// look glyphs up here.
//
// These carry no intrinsic size/stroke — the consuming surface's CSS owns that
// (e.g. `.settings-shell .nav-item svg`, `.onboarding-shell .icon-chip svg`),
// and CSS overrides the compiled svg's presentation attrs, so the icons match
// the surrounding 16px / stroke-1.7 family.
import type { Component } from "svelte";
import type { SvelteHTMLElements } from "svelte/elements";
import type { IconName } from "./groups";

import IconAppearance from "~icons/lucide/palette";
import IconStartup from "~icons/lucide/power";
import IconShortcuts from "~icons/lucide/keyboard";
import IconCapture from "~icons/lucide/monitor";
import IconVideo from "~icons/lucide/video";
import IconAudio from "~icons/lucide/audio-lines";
import IconPrivacy from "~icons/lucide/shield";
import IconIntelligence from "~icons/lucide/brain";
import IconAskAi from "~icons/lucide/sparkles";
import IconUserContext from "~icons/lucide/user";
import IconOcr from "~icons/lucide/scan-text";
import IconTranscription from "~icons/lucide/mic";
import IconSpeakers from "~icons/lucide/users";
import IconGpuAcceleration from "~icons/lucide/cpu";
import IconSemanticSearch from "~icons/lucide/search";
import IconStorage from "~icons/lucide/database";
import IconAccess from "~icons/lucide/key-round";
import IconAbout from "~icons/lucide/info";
import IconDeveloper from "~icons/lucide/code-xml";
import IconLock from "~icons/lucide/lock";

/** A Lucide icon as produced by unplugin-icons (`~icons/lucide/*`). */
export type IconComponent = Component<SvelteHTMLElements["svg"]>;

export const SECTION_ICONS: Record<IconName, IconComponent> = {
  appearance: IconAppearance,
  startup: IconStartup,
  shortcuts: IconShortcuts,
  capture: IconCapture,
  video: IconVideo,
  audio: IconAudio,
  privacy: IconPrivacy,
  intelligence: IconIntelligence,
  askAi: IconAskAi,
  userContext: IconUserContext,
  ocr: IconOcr,
  transcription: IconTranscription,
  speakers: IconSpeakers,
  gpuAcceleration: IconGpuAcceleration,
  semanticSearch: IconSemanticSearch,
  storage: IconStorage,
  access: IconAccess,
  about: IconAbout,
  developer: IconDeveloper,
  lock: IconLock,
};
