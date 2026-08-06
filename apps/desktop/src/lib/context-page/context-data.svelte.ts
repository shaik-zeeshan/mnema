// Context destination (direction 01, mockup 10) — the one data load behind the
// page, plus the four mutations the surface owns.
//
// No command here is new; they are the `#107` user-context commands the
// Insights Context sub-surface already uses. Every read is `catch`-guarded, so
// a disabled Reasoning Engine leaves the authored side of the page working —
// authored statements do not need an engine to be stored.
//
// Empty is ALWAYS `[]` / `null`, never an unresolved promise.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { humanizeError } from "$lib/format-error";
import type {
	AuthoredContext,
	Conclusion,
	DismissedView,
	UserContextStatus,
} from "$lib/types/recording";

/** How many inferred beliefs the "Steering your dossier" tile shows. */
const STEER_LIMIT = 3;

export class ContextData {
	/** False until the first burst settles — tiles read "Reading…" until then. */
	loaded = $state(false);

	statements = $state<AuthoredContext[]>([]);
	conclusions = $state<Conclusion[]>([]);
	dismissed = $state<DismissedView[]>([]);
	status = $state<UserContextStatus | null>(null);
	/** Set when the authored list itself failed to load (the one read whose
	 *  failure the page must not paper over as "you have nothing"). */
	loadError = $state<string | null>(null);

	async load(): Promise<void> {
		const [, conclusions, dismissed, status] = await Promise.all([
			invoke<AuthoredContext[]>("list_user_context_authored").then(
				(list) => {
					this.statements = list ?? [];
					this.loadError = null;
				},
				(error: unknown) => {
					this.loadError = humanizeError(error);
				},
			),
			invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }).catch(
				() => [],
			),
			invoke<DismissedView[]>("user_context_list_dismissed").catch(() => []),
			invoke<UserContextStatus>("get_user_context_status").catch(() => null),
		]);

		this.conclusions = conclusions ?? [];
		this.dismissed = dismissed ?? [];
		this.status = status ?? null;
		this.loaded = true;
	}

	/** Live for as long as the page is mounted: every user-context mutation —
	 *  ours or another surface's — re-reads the whole page. */
	watch(): () => void {
		let unlisten: UnlistenFn | undefined;
		let disposed = false;
		void listen("user_context_changed", () => void this.load()).then((fn) => {
			if (disposed) fn();
			else unlisten = fn;
		});
		return () => {
			disposed = true;
			unlisten?.();
		};
	}

	/** The beliefs this page reads out as "steering your dossier": the most
	 *  confident visible conclusions. They are INFERRED — the confidence belongs
	 *  to them, never to an authored statement (which carries none at all). */
	get steering(): Conclusion[] {
		return [...this.conclusions]
			.filter((c) => c.status === "visible")
			.sort((a, b) => b.confidence - a.confidence)
			.slice(0, STEER_LIMIT);
	}

	async add(text: string, topic: string | null): Promise<void> {
		const created = await invoke<AuthoredContext>("user_context_add_authored", { text, topic });
		this.statements = [created, ...this.statements];
	}

	async update(id: number, text: string, topic: string | null): Promise<void> {
		await invoke("user_context_update_authored", { id, text, topic });
		this.statements = this.statements.map((s) =>
			s.id === id ? { ...s, text, topic, updatedAtMs: Date.now() } : s,
		);
	}

	async remove(id: number): Promise<void> {
		await invoke("user_context_delete_authored", { id });
		this.statements = this.statements.filter((s) => s.id !== id);
	}

	async restore(d: DismissedView): Promise<void> {
		await invoke("user_context_restore_dismissed", {
			subject: d.subject,
			statement: d.statement,
		});
		this.dismissed = this.dismissed.filter(
			(x) => !(x.subject === d.subject && x.statement === d.statement),
		);
	}
}

/** "3w ago" — the same coarse ladder the Insights surface uses. */
export function relativeTime(ms: number): string {
	if (!Number.isFinite(ms) || ms <= 0) return "—";
	const diff = Date.now() - ms;
	if (diff < 0) return "just now";
	const min = Math.floor(diff / 60000);
	if (min < 1) return "just now";
	if (min < 60) return `${min}m ago`;
	const hr = Math.floor(min / 60);
	if (hr < 24) return `${hr}h ago`;
	const day = Math.floor(hr / 24);
	if (day < 7) return `${day}d ago`;
	const wk = Math.floor(day / 7);
	if (wk < 5) return `${wk}w ago`;
	const mo = Math.floor(day / 30);
	if (mo < 12) return `${mo}mo ago`;
	return `${Math.floor(day / 365)}y ago`;
}

/** "added 3w ago" / "edited 6d ago" — an update within a second of creation is
 *  the insert's own timestamp, not an edit. */
export function metaTime(s: AuthoredContext): string {
	return s.updatedAtMs > s.createdAtMs + 1000
		? `edited ${relativeTime(s.updatedAtMs)}`
		: `added ${relativeTime(s.createdAtMs)}`;
}
