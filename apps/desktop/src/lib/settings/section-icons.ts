// The settings rail's group glyphs (settings.html `.st-tile`), one Lucide
// component per `SettingsGroupId`.
//
// These carry no intrinsic size/stroke — the rail's CSS owns that, and CSS
// overrides the compiled svg's presentation attrs.
import type { Component } from "svelte";
import type { SvelteHTMLElements } from "svelte/elements";
import type { SettingsGroupId } from "./groups";

import IconCapture from "~icons/lucide/monitor";
import IconStorage from "~icons/lucide/database";
import IconAbout from "~icons/lucide/info";
import IconGeneral from "~icons/lucide/sliders-horizontal";
import IconSparkles from "~icons/lucide/sparkles";

/** A Lucide icon as produced by unplugin-icons (`~icons/lucide/*`). */
type IconComponent = Component<SvelteHTMLElements["svg"]>;

export const GROUP_ICONS: Record<SettingsGroupId, IconComponent> = {
  general: IconGeneral,
  capture: IconCapture,
  intelligence: IconSparkles,
  data: IconStorage,
  about: IconAbout,
};
