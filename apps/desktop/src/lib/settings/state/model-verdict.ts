// A model row's real output is a VERDICT, not a number (page 11: every model
// row carries "size · where it runs · a verdict computed against this Mac").
// The size and the machine denominators are already stated by
// `ui/ModelFootprintHint.svelte` (G8); this module answers the other half —
// where the thing runs, and whether it is usable here — from the status DTO
// the backend already sends. Nothing is invented: every branch below is a
// field, not a guess.
//
// Pure: no runes, no `invoke`.

import { formatBytes } from "./format";

export type VerdictTone = "ok" | "warn" | "bad" | "cloud";

export interface ModelVerdict {
	tone: VerdictTone;
	text: string;
}

/**
 * The providers whose work leaves this machine. Deepgram is the only one
 * (ADR 0047 — cloud transcription is a provider property), and it is gated by
 * an explicit consent dialog elsewhere; here it only has to be *named* as
 * cloud, so a blue verdict never hides behind a green one.
 */
const CLOUD_PROVIDERS = new Set(["deepgram"]);

export interface ModelVerdictInput {
	/** Provider id from the status DTO (`apple_vision`, `parakeet`, …). */
	provider: string;
	available: boolean;
	/** The DTO's status kind, verbatim. */
	status: string;
	/** The OS ships it — there is no download to have. */
	osManaged?: boolean;
	/** Download size in bytes when one is pending; null when there is none. */
	downloadBytes?: number | null;
}

export function modelVerdict(input: ModelVerdictInput): ModelVerdict {
	if (CLOUD_PROVIDERS.has(input.provider)) {
		return { tone: "cloud", text: "cloud · audio leaves this Mac" };
	}
	if (input.status === "failed") return { tone: "bad", text: "failed" };
	if (input.status === "downloading") return { tone: "warn", text: "downloading" };
	if (input.status === "incomplete") {
		return { tone: "warn", text: "incomplete · some files are missing" };
	}
	if (input.available) {
		return input.osManaged || input.status === "os_managed"
			? { tone: "ok", text: "runs on this Mac · no download" }
			: { tone: "ok", text: "installed · runs on this Mac" };
	}
	const bytes = input.downloadBytes ?? null;
	return bytes && bytes > 0
		? { tone: "warn", text: `not installed · ${formatBytes(bytes)} to download` }
		: { tone: "warn", text: "not installed" };
}
