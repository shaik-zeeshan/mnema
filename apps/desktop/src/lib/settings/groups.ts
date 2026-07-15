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
// IMPORTANT — behavior preservation: a deeplink reaches `resolveTabDeeplink`
// only AFTER passing through the upstream normalizers (`normalize_settings_tab`
// in Rust and `normalizeSettingsTab` in `surface-windows.ts`), so the alias set
// below is kept consistent with what those normalizers actually emit — any alias
// they drop to null can never arrive here. Keep the three in sync until the page
// is fully converted to the shell; `settings-groups.test.ts` and
// `settings-deeplink-aliases.test.ts` pin the verified deeplink expectations.

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
  | "gpuAcceleration"
  | "semanticSearch"
  | "storage"
  | "access"
  | "about"
  | "developer";

// The glyph identities the settings/onboarding surfaces draw. Every section has
// one; `lock` is the standalone "required/locked" glyph onboarding adds. The
// concrete Lucide components are mapped in `section-icons.ts`; this type stays
// here (framework-free) so `feature-model.ts` can type its `icon` field without
// importing Svelte.
export type IconName = SettingsSectionId | "lock";

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
  // Extra search terms the rail filter also matches (case-insensitive
  // substring), so queries users actually type for a setting that lives inside a
  // section ("theme", "retention", "bitrate") reach the right section even
  // though they aren't in its label. Optional; sections without searchable
  // settings can omit it.
  keywords?: string[];
  // Platform-gated section: only surfaced in the rail (and its search) on Windows.
  // Used by the Windows-only GPU Acceleration (NVIDIA CUDA backend) panel — macOS
  // has no GPU pack/toggle, so the section must be absent there entirely. The rail
  // filters these via `filterPlatform` (rail-filter.ts); the panel mirrors the same
  // `detectKeyboardPlatform()` guard so it renders nothing off Windows.
  windowsOnly?: boolean;
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
      {
        id: "appearance",
        anchor: "settings-section-appearance",
        label: "Appearance",
        keywords: ["theme", "dark", "light", "follow live recording"],
      },
      {
        id: "startup",
        anchor: "settings-section-startup",
        label: "Startup",
        keywords: ["auto-start", "launch", "login", "auto start recording"],
      },
      {
        id: "shortcuts",
        anchor: "settings-section-shortcuts",
        label: "Shortcuts",
        keywords: ["keyboard", "hotkey", "global shortcuts", "key binding"],
      },
    ],
  },
  {
    id: "capture",
    label: "Capture",
    description: "Sources, video, audio, privacy",
    sections: [
      {
        id: "capture",
        anchor: "settings-section-capture",
        label: "Capture",
        keywords: [
          "screen",
          "microphone",
          "system audio",
          "segment duration",
          "idle",
          "pause",
          "sensitivity",
          "vad",
          "voice detection",
        ],
      },
      {
        id: "video",
        anchor: "settings-section-video",
        label: "Video",
        keywords: ["bitrate", "resolution", "frame rate", "fps", "capture rate", "snapshot", "interval"],
      },
      {
        id: "audio",
        anchor: "settings-section-audio",
        label: "Audio",
        keywords: ["microphone", "device", "input", "on disconnect"],
      },
      {
        id: "privacy",
        anchor: "settings-section-privacy",
        label: "Privacy",
        keywords: ["excluded apps", "browser url", "frame context", "metadata"],
      },
    ],
  },
  {
    id: "intelligence",
    label: "Intelligence",
    description: "Providers, Ask AI, processing",
    sections: [
      {
        id: "intelligence",
        anchor: "settings-section-intelligence",
        label: "Providers",
        keywords: ["api key", "anthropic", "openai", "ollama", "default model", "ai runtime"],
      },
      {
        id: "askAi",
        anchor: "settings-section-askAi",
        label: "Ask AI",
        keywords: ["chat", "quick recall", "tool calls", "model override"],
      },
      {
        id: "userContext",
        anchor: "settings-section-userContext",
        label: "User Context",
        keywords: ["derive context", "derivation", "backfill", "subjects"],
      },
      {
        id: "ocr",
        anchor: "settings-section-ocr",
        label: "OCR",
        keywords: ["text recognition", "language", "preview cache", "tesseract"],
      },
      {
        id: "transcription",
        anchor: "settings-section-transcription",
        label: "Transcription",
        keywords: ["whisper", "parakeet", "audio", "transcribe", "model"],
      },
      {
        id: "speakers",
        anchor: "settings-section-speakers",
        label: "Speakers",
        keywords: ["diarization", "speaker separation", "recognize people"],
      },
      // Windows-only: the NVIDIA CUDA Execution Backend provisioning + Force-CPU
      // override (#137 / ADR 0005). `windowsOnly` keeps it out of the rail + search
      // on macOS, where speaker analysis is always CoreML (no pack, no toggle).
      // Placed right after Speakers (still BEFORE Semantic Search, so it is never the
      // group's tail section — the scroll-spy tail-fix keeps targeting semanticSearch).
      {
        id: "gpuAcceleration",
        anchor: "settings-section-gpuAcceleration",
        label: "GPU Acceleration",
        keywords: ["gpu", "cuda", "nvidia", "acceleration", "diarization"],
        windowsOnly: true,
      },
      {
        id: "semanticSearch",
        anchor: "settings-section-semanticSearch",
        label: "Semantic Search",
        keywords: ["embedding", "vector search", "model"],
      },
    ],
  },
  {
    id: "data",
    label: "Data",
    description: "Storage, CLI access",
    sections: [
      {
        id: "storage",
        anchor: "settings-section-storage",
        label: "Storage",
        keywords: ["retention", "save directory", "storage location", "disk"],
      },
      {
        id: "access",
        anchor: "settings-section-access",
        label: "Access",
        keywords: ["cli access", "agent", "broker"],
      },
    ],
  },
  {
    id: "about",
    label: "About",
    description: "Version, updates, developer",
    sections: [
      {
        id: "about",
        anchor: "settings-section-about",
        label: "About",
        keywords: ["version", "update channel", "third-party notices", "acknowledgements"],
      },
      {
        id: "developer",
        anchor: "settings-section-developer",
        label: "Developer",
        keywords: ["logs", "debug logging", "developer options"],
      },
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

// ── Deeplink resolution (downstream of the upstream normalizers) ─────────────
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
    case "semanticSearch":
    case "semantic-search":
      return "semanticSearch";
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
