// The debug page's section registry — one source of truth for the dock order,
// the scroll anchors, and the health-dot mapping.
//
// The dock renders this list top-to-bottom and a click scrolls to the matching
// `anchor(id)` element; every section component must therefore hand its id to
// `<SettingGroup id={anchor("...")}>` (the anchor lives on SettingGroup's outer
// <section>, never on the inner card).
//
// `healthFeature` is nullable on purpose: `get_debug_health` returns **9**
// features but there are **11** sections — Health (it *is* the rollup) and Logs
// (nothing to roll up) have no dot of their own, so they render `idle`.
//
// `group` splits the dock with a separator wherever the number changes.

import type { Component } from "svelte";
import IconActivity from "~icons/lucide/activity";
import IconMonitor from "~icons/lucide/monitor";
import IconShield from "~icons/lucide/shield";
import IconScanText from "~icons/lucide/scan-text";
import IconMic from "~icons/lucide/mic";
import IconUsers from "~icons/lucide/users";
import IconGrid from "~icons/lucide/grid-3x3";
import IconSparkles from "~icons/lucide/sparkles";
import IconUser from "~icons/lucide/user";
import IconDatabase from "~icons/lucide/database";
import IconFileText from "~icons/lucide/file-text";
import type { DebugFeature } from "$lib/types";

export type DebugSectionId =
	| "health"
	| "capture"
	| "privacy"
	| "ocr"
	| "transcription"
	| "diarization"
	| "embeddings"
	| "aiRuntime"
	| "userContext"
	| "jobsAndStorage"
	| "logs";

export type DebugSection = {
	id: DebugSectionId;
	/** Dock tooltip + section title. */
	label: string;
	/** Monochrome Lucide glyph — never emoji (colour emoji clashed with the theme). */
	icon: Component;
	/** The `get_debug_health` feature backing this section's dot, if any. */
	healthFeature: DebugFeature | null;
	/** Dock group; a separator is drawn between differing values. */
	group: number;
};

export const DEBUG_SECTIONS: DebugSection[] = [
	{ id: "health", label: "Health", icon: IconActivity, healthFeature: null, group: 0 },
	{ id: "capture", label: "Capture Sources", icon: IconMonitor, healthFeature: "capture", group: 0 },

	{ id: "privacy", label: "Privacy & Inactivity", icon: IconShield, healthFeature: "privacy", group: 1 },
	{ id: "ocr", label: "OCR", icon: IconScanText, healthFeature: "ocr", group: 1 },

	{ id: "transcription", label: "Transcription", icon: IconMic, healthFeature: "transcription", group: 2 },

	{ id: "diarization", label: "Diarization", icon: IconUsers, healthFeature: "diarization", group: 3 },

	{ id: "embeddings", label: "Embeddings", icon: IconGrid, healthFeature: "embeddings", group: 4 },
	{ id: "aiRuntime", label: "AI Runtime", icon: IconSparkles, healthFeature: "aiRuntime", group: 4 },

	{ id: "userContext", label: "User Context", icon: IconUser, healthFeature: "userContext", group: 5 },
	{ id: "jobsAndStorage", label: "Jobs & Storage", icon: IconDatabase, healthFeature: "jobsAndStorage", group: 5 },

	{ id: "logs", label: "Logs", icon: IconFileText, healthFeature: null, group: 6 },
];

/** The DOM id for a section — the dock's scroll target. */
export function anchor(id: DebugSectionId): string {
	return `debug-section-${id}`;
}
