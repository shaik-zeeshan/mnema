// Pure Settings rail grouping + deeplink resolution.
//
// Stage-1 shell-ification (Slice 5) regroups the 12 legacy settings sections
// into 5 navigation groups. This module is the single source of truth for:
//   • which sections belong to which group (and in what order they stack),
//   • the human labels/descriptions/anchor ids the rail + panels render,
//   • how an incoming `?tab=` / `?focus=` deeplink resolves to a (group,
//     section-anchor) pair so the shell can select the group and scroll to the
//     right section.
//
// It is intentionally framework-free (no `$state`/Svelte) so it is unit-testable
// and reusable by both `SettingsRail.svelte` and the shell `+page.svelte`. The
// section ids match the existing per-section panel anchor ids
// (`settings-panel-<section>`) so scroll-to-anchor keeps working unchanged.
//
// IMPORTANT — behavior preservation: the deeplink alias set below is a 1:1 port
// of the legacy `normalizeSettingsTab`/`normalizeSettingsFocus` in
// `routes/settings/+page.svelte`. Keep the two in sync until the page is fully
// converted to the shell; the `settings-groups.test.ts` spec pins the verified
// deeplink expectations.

// The 12 legacy sections (each had its own `{#if activeTab === ...}` block and a
// `settings-panel-<id>` anchor). These are the scroll targets within a group.
export type SettingsSectionId =
  | "appearance"
  | "startup"
  | "shortcuts"
  | "capture"
  | "video"
  | "audio"
  | "privacy"
  | "intelligence"
  | "askAi"
  | "userContext"
  | "ocr"
  | "transcription"
  | "speakers"
  | "semanticSearch"
  | "storage"
  | "access"
  | "about"
  | "developer";

// The 5 navigation groups the rail renders.
export type SettingsGroupId =
  | "general"
  | "capture"
  | "intelligence"
  | "data"
  | "about";

export interface SettingsSection {
  id: SettingsSectionId;
  // The DOM anchor the shell scrolls to. Matches `settings-panel-<anchor>` for
  // sections that already had a panel id; new sub-section anchors get their own.
  anchor: string;
  label: string;
}

export interface SettingsGroup {
  id: SettingsGroupId;
  label: string;
  description: string;
  sections: SettingsSection[];
}

// ── The 5 groups, in rail order, each stacking its sections in render order ──
//
//  • General:      Appearance, Startup, Shortcuts
//  • Capture:      Capture, Video, Audio, Privacy
//  • Intelligence: Providers, Ask AI, User Context, OCR, Transcription,
//                  Speakers, Semantic Search
//  • Data:         Storage, Access
//  • About:        About, Developer
export const SETTINGS_GROUPS: readonly SettingsGroup[] = [
  {
    id: "general",
    label: "General",
    description: "Appearance, startup, shortcuts",
    sections: [
      { id: "appearance", anchor: "settings-section-appearance", label: "Appearance" },
      { id: "startup", anchor: "settings-section-startup", label: "Startup" },
      { id: "shortcuts", anchor: "settings-section-shortcuts", label: "Shortcuts" },
    ],
  },
  {
    id: "capture",
    label: "Capture",
    description: "Sources, video, audio, privacy",
    sections: [
      { id: "capture", anchor: "settings-section-capture", label: "Capture" },
      { id: "video", anchor: "settings-section-video", label: "Video" },
      { id: "audio", anchor: "settings-section-audio", label: "Audio" },
      { id: "privacy", anchor: "settings-section-privacy", label: "Privacy" },
    ],
  },
  {
    id: "intelligence",
    label: "Intelligence",
    description: "Providers, Ask AI, processing",
    sections: [
      { id: "intelligence", anchor: "settings-section-intelligence", label: "Providers" },
      { id: "askAi", anchor: "settings-section-askAi", label: "Ask AI" },
      { id: "userContext", anchor: "settings-section-userContext", label: "User Context" },
      { id: "ocr", anchor: "settings-section-ocr", label: "OCR" },
      { id: "transcription", anchor: "settings-section-transcription", label: "Transcription" },
      { id: "speakers", anchor: "settings-section-speakers", label: "Speakers" },
      { id: "semanticSearch", anchor: "settings-section-semanticSearch", label: "Semantic Search" },
    ],
  },
  {
    id: "data",
    label: "Data",
    description: "Storage, CLI access",
    sections: [
      { id: "storage", anchor: "settings-section-storage", label: "Storage" },
      { id: "access", anchor: "settings-section-access", label: "Access" },
    ],
  },
  {
    id: "about",
    label: "About",
    description: "Version, updates, developer",
    sections: [
      { id: "about", anchor: "settings-section-about", label: "About" },
      { id: "developer", anchor: "settings-section-developer", label: "Developer" },
    ],
  },
];

// The group + section the Settings shell lands on when opened with no
// ?tab/?focus deeplink: the first rail group (General) and its first section
// (Appearance). Derived from rail order so a reorder updates the default;
// pinned by settings-groups.test.ts.
export const DEFAULT_SETTINGS_GROUP: SettingsGroupId = SETTINGS_GROUPS[0].id;
export const DEFAULT_SETTINGS_SECTION: SettingsSectionId = SETTINGS_GROUPS[0].sections[0].id;

// Reverse index: section id → its owning group id.
const SECTION_TO_GROUP: Record<SettingsSectionId, SettingsGroupId> = (() => {
  const map = {} as Record<SettingsSectionId, SettingsGroupId>;
  for (const group of SETTINGS_GROUPS) {
    for (const section of group.sections) map[section.id] = group.id;
  }
  return map;
})();

export function groupForSection(section: SettingsSectionId): SettingsGroupId {
  return SECTION_TO_GROUP[section];
}

export function sectionAnchor(section: SettingsSectionId): string {
  for (const group of SETTINGS_GROUPS) {
    const found = group.sections.find((s) => s.id === section);
    if (found) return found.anchor;
  }
  return `settings-section-${section}`;
}

// ── Deeplink resolution (1:1 port of the legacy normalizers) ─────────────────
//
// Resolve a `?tab=` value (or legacy alias) to the section it should land on.
// The shell maps that section to its group via `groupForSection` and scrolls to
// `sectionAnchor`. Returns null for an unknown value (the shell keeps the
// current group, matching the legacy "no-op on unknown tab" behavior).
export function resolveTabDeeplink(
  value: string | null | undefined,
): SettingsSectionId | null {
  switch (value) {
    case "about":
      return "about";
    // Legacy "behavior" alias mapped to the capture tab.
    case "capture":
    case "behavior":
      return "capture";
    case "access":
    case "cliAccess":
    case "cli-access":
      return "access";
    // The legacy "intelligence" tab stacked providers + Ask AI + User Context;
    // its aliases land on the Providers section of the Intelligence group.
    case "intelligence":
    case "reasoning":
    case "reasoning-engine":
    case "ai":
    case "ai-runtime":
      return "intelligence";
    case "user-context":
    case "userContext":
      return "userContext";
    case "privacy":
    case "metadata":
      return "privacy";
    case "shortcuts":
    case "keyboard":
    case "keyboard-shortcuts":
    case "keyboard_bindings":
      return "shortcuts";
    case "video":
      return "video";
    case "audio":
    case "microphone":
      return "audio";
    // The legacy "processing" tab stacked OCR + transcription + speakers +
    // semantic search; its sub-tab aliases now land on the matching Intelligence
    // section directly.
    case "processing":
    case "ocr":
      return "ocr";
    case "transcription":
      return "transcription";
    case "speakers":
      return "speakers";
    case "storage":
      return "storage";
    case "appearance":
      return "appearance";
    case "developer":
      return "developer";
    default:
      return null;
  }
}

export type SettingsFocusTarget = "cliAccess";

// Resolve a `?focus=` value to a focus target (1:1 port of the legacy
// `normalizeSettingsFocus`). `cliAccess` lives in the Data group's Access
// section and additionally pops the broker-authorization prompt.
export function resolveFocusDeeplink(
  value: string | null | undefined,
): SettingsFocusTarget | null {
  switch (value) {
    case "agentAccess":
    case "agent-access":
    case "cliAccess":
    case "cli-access":
      return "cliAccess";
    default:
      return null;
  }
}

// The section a focus target scrolls to (Access, in the Data group).
export function sectionForFocus(focus: SettingsFocusTarget): SettingsSectionId {
  switch (focus) {
    case "cliAccess":
      return "access";
  }
}
