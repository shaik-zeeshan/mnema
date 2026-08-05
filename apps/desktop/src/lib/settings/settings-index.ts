// The Settings row index (DECISIONS.md G7) — every settings row, with the extra
// words a user might type for it.
//
// ⌘F inside /settings filters the content pane down to matching rows (rendered
// in place, with their live control). This module is the framework-free half:
// the index itself plus the matcher. The reactive half lives in
// `state/settings-find.svelte.ts`; `ui/SettingRow.svelte` renders it.
//
// COMPLETENESS IS ENFORCED BY A TEST. `specs/settings-row-index.test.ts` reads every
// `<SettingRow label="…">` out of `panels/**/*.svelte` (mapping each file to its
// section via the `setSettingsSection("…")` call it makes) and fails if a row
// has no entry here. A new row cannot dodge the index silently — the label in
// the markup is the enumerable source of truth, and this file must cover it.
//
// SYNONYMS: only the words the label + section name DON'T already carry. The
// matcher already searches the label, the section label and the group label, so
// "Retention" needs no "retention" synonym — it needs "delete old", "days".

import { SETTINGS_GROUPS, type SettingsSectionId } from "./groups";

export interface SettingsRowIndexEntry {
	/** The section the row lives in (its `setSettingsSection` scope). */
	section: SettingsSectionId;
	/** The row's rendered label — the key the markup is checked against. */
	label: string;
	/** Extra search terms. Words the label/section already carry are redundant. */
	synonyms?: string[];
}

// ── The index ───────────────────────────────────────────────────────────────
// Grouped by section, in rail order. Keep a section's rows in render order so a
// diff against the panel file reads straight down.
export const SETTINGS_ROW_INDEX: readonly SettingsRowIndexEntry[] = [
	// ── General ──
	{ section: "appearance", label: "Theme", synonyms: ["dark", "light", "system", "color scheme", "appearance"] },
	{ section: "appearance", label: "Follow live recording", synonyms: ["auto scroll", "timeline", "live"] },
	{ section: "startup", label: "Open Mnema on", synonyms: ["startup surface", "landing", "first screen", "timeline", "overview"] },
	{ section: "startup", label: "Auto-start recording on launch", synonyms: ["login", "launch", "open at login", "automatic"] },
	{ section: "shortcuts", label: "Global shortcuts", synonyms: ["hotkey", "key binding", "rebind", "keyboard"] },

	// ── Capture ──
	{ section: "capture", label: "Capture", synonyms: ["sources", "recording", "what is recorded"] },
	{ section: "capture", label: "Screen", synonyms: ["display", "video", "screen recording"] },
	{ section: "capture", label: "Microphone", synonyms: ["mic", "audio input", "voice"] },
	{ section: "capture", label: "System Audio", synonyms: ["desktop audio", "speaker audio", "process tap"] },
	{ section: "capture", label: "System audio access", synonyms: ["permission", "authorization", "tcc"] },
	{ section: "capture", label: "Segment Duration", synonyms: ["chunk", "file length", "minutes", "split"] },
	{ section: "capture", label: "Pause capture when idle", synonyms: ["inactivity", "auto pause", "away"] },
	{ section: "capture", label: "Idle timeout", synonyms: ["inactivity", "minutes", "auto pause"] },
	{ section: "capture", label: "Activity sources", synonyms: ["idle detection", "keyboard", "mouse", "input"] },
	{ section: "capture", label: "Microphone Voice Detection", synonyms: ["vad", "voice activity", "silence"] },
	{ section: "capture", label: "Microphone Activity Sensitivity", synonyms: ["vad", "threshold", "silence"] },
	{ section: "capture", label: "System Audio Activity Sensitivity", synonyms: ["vad", "threshold", "silence"] },
	{ section: "capture", label: "Audio Activity Monitoring", synonyms: ["vad", "levels", "meter"] },
	{ section: "capture", label: "Validation", synonyms: ["errors", "problems"] },
	{ section: "video", label: "Screen Capture Rate", synonyms: ["fps", "frame rate", "snapshot", "interval"] },
	// Retention lives under Capture › Video, not Data — a stated direction-01 IA
	// deviation: it is the second half of the frame-rate decision.
	{ section: "video", label: "Retention", synonyms: ["delete old", "keep", "days", "cleanup", "disk", "storage"] },
	{ section: "video", label: "Screen Resolution", synonyms: ["size", "scaling", "1080p", "custom dimensions"] },
	{ section: "video", label: "Bitrate", synonyms: ["quality", "mbps", "compression", "file size"] },
	{ section: "audio", label: "Microphone", synonyms: ["mic", "enable microphone"] },
	{ section: "audio", label: "Active Device", synonyms: ["mic", "input device", "current"] },
	{ section: "audio", label: "Available Devices", synonyms: ["mic", "input device", "list"] },
	{ section: "audio", label: "Preference", synonyms: ["mic", "preferred device", "default input"] },
	{ section: "audio", label: "Device", synonyms: ["mic", "input device"] },
	{ section: "audio", label: "On Disconnect", synonyms: ["unplug", "fallback", "device lost"] },
	{ section: "audio", label: "Error", synonyms: ["mic", "problem", "failure"] },
	{ section: "privacy", label: "Capture frame context", synonyms: ["window title", "metadata", "app name"] },
	{ section: "privacy", label: "Browser URL mode", synonyms: ["web address", "browsing", "safari", "chrome"] },
	{ section: "privacy", label: "Browser URL access (Firefox / Zen)", synonyms: ["accessibility", "gecko", "permission"] },
	{ section: "privacy", label: "Excluded Apps", synonyms: ["exclusion", "blocklist", "hide app", "1password", "sensitive"] },
	{ section: "privacy", label: "Filter system audio", synonyms: ["exclude audio", "tap exclude", "mute app"] },

	// ── Intelligence ──
	{ section: "intelligence", label: "Enable AI features", synonyms: ["ai", "llm", "reasoning"] },
	{ section: "intelligence", label: "Providers", synonyms: ["api key", "anthropic", "openai", "ollama", "llamafile"] },
	{ section: "intelligence", label: "Global default model", synonyms: ["default model", "llm"] },
	{ section: "intelligence", label: "AI runtime", synonyms: ["status", "engine", "reachable"] },
	{ section: "intelligence", label: "Connectors", synonyms: ["mcp", "tools", "integrations", "server"] },
	{ section: "askAi", label: "Enable Ask AI", synonyms: ["chat", "quick recall", "assistant"] },
	{ section: "askAi", label: "Fetch pages you visited", synonyms: ["web fetch", "url", "browsing"] },
	{ section: "askAi", label: "Limit tool calls per question", synonyms: ["tool budget", "max steps", "cost"] },
	{ section: "askAi", label: "Model override", synonyms: ["model", "per feature model"] },
	{ section: "userContext", label: "Derive context continuously", synonyms: ["distillation", "subjects", "beliefs"] },
	{ section: "userContext", label: "Derivation status", synonyms: ["progress", "queue"] },
	{ section: "userContext", label: "Derivation Budget", synonyms: ["cost", "tokens", "spend"] },
	{ section: "userContext", label: "History Backfill", synonyms: ["past", "catch up", "reprocess"] },
	{ section: "userContext", label: "Run derivation", synonyms: ["run now", "manual"] },
	{ section: "ocr", label: "Enable OCR", synonyms: ["text recognition", "screen text"] },
	{ section: "ocr", label: "Pacing", synonyms: ["duty cycle", "throttle", "cooldown", "cpu", "backlog", "queue"] },
	{ section: "ocr", label: "Provider", synonyms: ["engine", "tesseract", "apple vision"] },
	{ section: "ocr", label: "Model", synonyms: ["download", "weights"] },
	{ section: "ocr", label: "Language", synonyms: ["languages", "locale"] },
	{ section: "ocr", label: "Recognition mode", synonyms: ["accurate", "fast", "quality"] },
	{ section: "ocr", label: "Language correction", synonyms: ["autocorrect", "dictionary"] },
	{ section: "ocr", label: "Page segmentation", synonyms: ["psm", "layout"] },
	{ section: "ocr", label: "Image preprocessing", synonyms: ["threshold", "contrast", "binarize"] },
	{ section: "ocr", label: "Upscale before OCR", synonyms: ["scale", "resize", "small text"] },
	{ section: "ocr", label: "Character whitelist", synonyms: ["charset", "allowed characters"] },
	{ section: "ocr", label: "OCR availability", synonyms: ["status", "installed"] },
	{ section: "ocr", label: "Selected model status", synonyms: ["download", "missing files"] },
	{ section: "ocr", label: "Cache duration", synonyms: ["preview cache", "frames", "disk"] },
	{ section: "transcription", label: "Enable audio transcription", synonyms: ["speech to text", "subtitles"] },
	{ section: "transcription", label: "Transcribe microphone", synonyms: ["mic", "speech to text"] },
	{ section: "transcription", label: "Transcribe system audio", synonyms: ["desktop audio", "speech to text"] },
	{ section: "transcription", label: "Provider", synonyms: ["whisper", "parakeet", "deepgram", "cloud", "engine"] },
	{ section: "transcription", label: "Model", synonyms: ["download", "whisper", "parakeet"] },
	{ section: "transcription", label: "Language", synonyms: ["languages", "locale"] },
	{ section: "transcription", label: "Deepgram API key", synonyms: ["cloud key", "secret", "token"] },
	{ section: "transcription", label: "Parakeet memory mode", synonyms: ["low memory", "ram", "footprint"] },
	{ section: "transcription", label: "Idle unload seconds", synonyms: ["memory", "ram", "unload"] },
	{ section: "transcription", label: "Chunk seconds", synonyms: ["window", "batch", "length"] },
	{ section: "transcription", label: "Selected model status", synonyms: ["download", "missing files"] },
	{ section: "speakers", label: "Speaker separation", synonyms: ["diarization", "who spoke"] },
	{ section: "speakers", label: "Helper timeout", synonyms: ["diarization", "seconds", "give up"] },
	{ section: "speakers", label: "Speaker model", synonyms: ["speakrs", "diarization model", "download"] },
	{ section: "speakers", label: "Voiceprint", synonyms: ["my voice", "enroll", "enrollment", "recognize me"] },
	{ section: "speakers", label: "Label my voice automatically", synonyms: ["recognition", "identify me"] },
	{ section: "semanticSearch", label: "Enable semantic search", synonyms: ["embeddings", "vector", "meaning"] },
	{ section: "semanticSearch", label: "Model", synonyms: ["embedding model", "download", "nomic"] },

	// ── Data ──
	{ section: "storage", label: "Save Directory", synonyms: ["location", "path", "folder", "where"] },
	{ section: "access", label: "CLI Access", synonyms: ["agent", "broker", "mnema-cli", "grant", "terminal"] },

	// ── About ──
	{ section: "license", label: "Status", synonyms: ["license", "trial", "activated"] },
	{ section: "license", label: "Refresh license status", synonyms: ["recheck", "sync"] },
	{ section: "license", label: "Buy Mnema", synonyms: ["purchase", "pay", "license"] },
	{ section: "license", label: "Renew", synonyms: ["update window", "extend", "subscription"] },
	{ section: "license", label: "Activate license", synonyms: ["key", "activation", "redeem"] },
	{ section: "about", label: "Copy details", synonyms: ["version", "build", "diagnostics", "support"] },
	{ section: "about", label: "Update channel", synonyms: ["stable", "preview", "beta", "updates"] },
	{ section: "about", label: "Confirm preview channel", synonyms: ["beta", "opt in"] },
	{ section: "about", label: "Status", synonyms: ["update check", "version"] },
	{ section: "about", label: "Third-party notices", synonyms: ["licenses", "acknowledgements", "open source"] },
	{ section: "developer", label: "Enable developer options", synonyms: ["debug", "advanced"] },
	{ section: "developer", label: "Native capture debug logging", synonyms: ["logs", "verbose", "trace"] },
	{ section: "developer", label: "Native capture log", synonyms: ["logs", "rust log", "file"] },
	{ section: "developer", label: "General application log", synonyms: ["logs", "file"] },
];

// ── Lookup + breadcrumb ─────────────────────────────────────────────────────

function key(section: SettingsSectionId, label: string): string {
	return `${section} ${label}`;
}

const BY_KEY: ReadonlyMap<string, SettingsRowIndexEntry> = new Map(
	SETTINGS_ROW_INDEX.map((entry) => [key(entry.section, entry.label), entry]),
);

/** The index entry for a rendered row, or null when it has none (test-enforced). */
export function rowIndexEntry(
	section: SettingsSectionId | null,
	label: string,
): SettingsRowIndexEntry | null {
	if (section === null) return null;
	return BY_KEY.get(key(section, label)) ?? null;
}

/** Section id → the labels ⌘F shows as the hit's breadcrumb ("Capture › Video"). */
export function sectionBreadcrumb(
	section: SettingsSectionId,
): { group: string; section: string } | null {
	for (const group of SETTINGS_GROUPS) {
		const found = group.sections.find((s) => s.id === section);
		if (found) return { group: group.label, section: found.label };
	}
	return null;
}

// ── The matcher ─────────────────────────────────────────────────────────────
//
// ponytail: case-insensitive substring, AND-ed across whitespace-separated
// tokens, over one haystack = label + synonyms + section label + group label.
// That already answers "ocr lang" and "video fps"; a real fuzzy/edit-distance
// ranker is speculative until someone misses a hit they typed correctly.

function haystack(section: SettingsSectionId | null, label: string): string {
	const parts = [label];
	const entry = rowIndexEntry(section, label);
	if (entry?.synonyms) parts.push(...entry.synonyms);
	if (section) {
		const crumb = sectionBreadcrumb(section);
		if (crumb) parts.push(crumb.section, crumb.group);
	}
	return parts.join(" ").toLowerCase();
}

/**
 * Does this row match the ⌘F query?
 *
 * Matches on the row label, its indexed synonyms, its section label and its
 * group label — case-insensitive substring, every query token required.
 * An empty / whitespace-only query matches NOTHING (the filter is only ever
 * consulted while a query is active; callers render the normal panels instead).
 */
export function rowMatchesQuery(
	section: SettingsSectionId | null,
	label: string,
	query: string,
): boolean {
	const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (tokens.length === 0) return false;
	const hay = haystack(section, label);
	return tokens.every((token) => hay.includes(token));
}
