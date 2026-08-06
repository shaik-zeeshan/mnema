// Context — the destination's one reader and its four mutations.
//
// This surface owns exactly ONE kind of data: **authored context**, the
// sentences the user wrote about themselves. Everything Mnema *inferred* lives
// on Subjects. The only other thing drawn here is the dismissed archive, which
// is inferred beliefs the user removed — it is on this page because restoring
// one is a correction of the same kind as editing a line you wrote.
//
// The backend commands already exist (issue #107); this file is a thin, honest
// wrapper over them, with the optimistic-update shapes taken from the old
// `lib/insights/Context.svelte`:
//
//   list_user_context_authored        → AuthoredContext[] (newest first)
//   user_context_add_authored         { text, topic } → AuthoredContext
//   user_context_update_authored      { id, text, topic } → void
//   user_context_delete_authored      { id } → void
//   user_context_list_dismissed       → DismissedView[]
//   user_context_restore_dismissed    { subject, statement } → void
//   get_user_context_status           → UserContextStatus
//   list_user_context_conclusions     { includeFaded } → Conclusion[]
//
// Every list re-loads on the `user_context_changed` event, so a derivation pass
// or another surface's edit lands here without a refresh.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { humanizeError } from "$lib/format-error";
import { statusSave } from "$lib/studio/status-strip.svelte";
import type {
	AuthoredContext,
	Conclusion,
	DismissedView,
	UserContextStatus,
} from "$lib/types/recording";

/** The composer's chips. They PREFILL an opening — they are not categories. */
export const SUGGESTIONS: { label: string; prompt: string }[] = [
	{ label: "Your role", prompt: "I'm a … " },
	{ label: "What you're working on", prompt: "I'm currently working on … " },
	{ label: "How you work", prompt: "I prefer to work by … " },
	{ label: "What you care about", prompt: "I care deeply about … " },
	{ label: "Goals this quarter", prompt: "Goal: " },
];

/** Coarse age, in the ledger's "added 2h ago" form. */
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

/** "added 2h ago" / "edited 3d ago" — a line is edited when it moved after birth. */
export function metaTime(s: AuthoredContext): string {
	return s.updatedAtMs > s.createdAtMs + 1000
		? `edited ${relativeTime(s.updatedAtMs)}`
		: `added ${relativeTime(s.createdAtMs)}`;
}

export function dismissedKey(d: DismissedView): string {
	return `${d.subject}\0${d.statement}`;
}

/** What the inspector is currently about. */
export type Focus =
	| { kind: "none" }
	| { kind: "authored"; item: AuthoredContext }
	| { kind: "dismissed"; item: DismissedView }
	| { kind: "editing"; item: AuthoredContext };

/**
 * One authored line and an inferred belief that carries the same subject.
 *
 * This is the ONLY linkage the data has. `user_context_authored.topic` is
 * documented in migration 0026 as "mirrors a Conclusion's Subject", and a
 * Conclusion's `evidence` points at Activities, never at an authored line —
 * nothing records which sentence shaped which belief. So the inspector pairs
 * them by that shared handle and says so; it never claims causation.
 */
export interface SteerLink {
	topic: string;
	statement: string;
	confidence: number;
}

export class ContextData {
	/** null until the first list lands. */
	statements = $state<AuthoredContext[] | null>(null);
	loadError = $state<string | null>(null);
	loading = $state(true);

	dismissed = $state<DismissedView[] | null>(null);
	dismissedError = $state<string | null>(null);
	/** Collapsed by default — the archive is the page's basement, not its floor. */
	showDismissed = $state(false);
	restoringKey = $state<string | null>(null);

	status = $state<UserContextStatus | null>(null);
	conclusions = $state<Conclusion[] | null>(null);

	focus = $state<Focus>({ kind: "none" });

	/** The id whose row is showing its "Saved ✓" echo, cleared after ~1.5s. */
	echoId = $state<number | null>(null);
	#echoTimer: ReturnType<typeof setTimeout> | null = null;

	readonly standingCount = $derived(this.statements?.length ?? null);
	readonly dismissedCount = $derived(this.dismissed?.length ?? null);

	/**
	 * Authored topics that share a subject with a live inferred belief. Empty
	 * when nothing matches — the inspector then omits the section rather than
	 * inventing an edge (see `SteerLink`).
	 */
	readonly steerLinks = $derived.by<SteerLink[]>(() => {
		const lines = this.statements;
		const beliefs = this.conclusions;
		if (!lines || !beliefs) return [];
		const bySubject = new Map<string, Conclusion>();
		for (const c of beliefs) {
			if (c.status !== "visible") continue;
			const key = c.subject.trim().toLowerCase();
			const seen = bySubject.get(key);
			if (!seen || c.confidence > seen.confidence) bySubject.set(key, c);
		}
		const out: SteerLink[] = [];
		const used = new Set<string>();
		for (const s of lines) {
			const topic = s.topic?.trim();
			if (!topic) continue;
			const key = topic.toLowerCase();
			if (used.has(key)) continue;
			const match = bySubject.get(key);
			if (!match) continue;
			used.add(key);
			out.push({ topic, statement: match.statement, confidence: match.confidence });
			if (out.length === 3) break;
		}
		return out;
	});

	// ── Reads ────────────────────────────────────────────────────────────

	async loadStatements(): Promise<void> {
		this.loading = true;
		try {
			this.statements = await invoke<AuthoredContext[]>("list_user_context_authored");
			this.loadError = null;
		} catch (error) {
			this.loadError = humanizeError(error);
			this.statements = this.statements ?? [];
		} finally {
			this.loading = false;
		}
	}

	async loadDismissed(): Promise<void> {
		try {
			this.dismissed = await invoke<DismissedView[]>("user_context_list_dismissed");
			this.dismissedError = null;
		} catch (error) {
			this.dismissedError = humanizeError(error);
			this.dismissed = this.dismissed ?? [];
		}
	}

	/** Best-effort: the tool strip's derivation chip and the steering links. */
	async loadSide(): Promise<void> {
		const [status, conclusions] = await Promise.all([
			invoke<UserContextStatus>("get_user_context_status").catch(() => null),
			invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }).catch(
				() => null,
			),
		]);
		if (status) this.status = status;
		if (conclusions) this.conclusions = conclusions;
	}

	/** Mount: load everything, then keep it live. Returns the teardown. */
	start(): () => void {
		void this.loadStatements();
		void this.loadDismissed();
		void this.loadSide();

		let unlisten: UnlistenFn | undefined;
		let disposed = false;
		void listen("user_context_changed", () => {
			void this.loadStatements();
			void this.loadDismissed();
			void this.loadSide();
		}).then((fn) => {
			if (disposed) fn();
			else unlisten = fn;
		});

		return () => {
			disposed = true;
			unlisten?.();
			if (this.#echoTimer) clearTimeout(this.#echoTimer);
			statusSave.set(null);
		};
	}

	// ── Writes ───────────────────────────────────────────────────────────

	/** G7's autosave pattern: the strip says *whether*, the row says *what*. */
	#saved(id: number | null): void {
		statusSave.set({
			tone: "ok",
			label: `All changes saved · ${new Date().toLocaleTimeString(undefined, {
				hour: "2-digit",
				minute: "2-digit",
			})}`,
		});
		if (id === null) return;
		this.echoId = id;
		if (this.#echoTimer) clearTimeout(this.#echoTimer);
		this.#echoTimer = setTimeout(() => (this.echoId = null), 1500);
	}

	#failed(error: unknown): string {
		const detail = humanizeError(error);
		statusSave.set({ tone: "bad", label: "Couldn't save" });
		return detail;
	}

	/** Returns null on success, the error message otherwise. */
	async add(text: string, topic: string): Promise<string | null> {
		statusSave.set({ tone: "busy", label: "Saving…" });
		try {
			const created = await invoke<AuthoredContext>("user_context_add_authored", {
				text,
				topic: topic.length > 0 ? topic : null,
			});
			this.statements = [created, ...(this.statements ?? [])];
			this.#saved(created.id);
			return null;
		} catch (error) {
			return this.#failed(error);
		}
	}

	async update(id: number, text: string, topic: string): Promise<string | null> {
		const nextTopic = topic.length > 0 ? topic : null;
		statusSave.set({ tone: "busy", label: "Saving…" });
		try {
			await invoke("user_context_update_authored", { id, text, topic: nextTopic });
			this.statements = (this.statements ?? []).map((s) =>
				s.id === id ? { ...s, text, topic: nextTopic, updatedAtMs: Date.now() } : s,
			);
			this.#saved(id);
			return null;
		} catch (error) {
			return this.#failed(error);
		}
	}

	async remove(id: number): Promise<string | null> {
		statusSave.set({ tone: "busy", label: "Saving…" });
		try {
			await invoke("user_context_delete_authored", { id });
			this.statements = (this.statements ?? []).filter((s) => s.id !== id);
			const f = this.focus;
			if ((f.kind === "authored" || f.kind === "editing") && f.item.id === id) {
				this.focus = { kind: "none" };
			}
			this.#saved(null);
			return null;
		} catch (error) {
			return this.#failed(error);
		}
	}

	/**
	 * Restore clears the dismissal so the belief is ALLOWED to form again on the
	 * next derivation pass. It does not put the old conclusion back — if the
	 * activity no longer supports it, nothing returns, and that is correct.
	 */
	async restore(d: DismissedView): Promise<void> {
		const key = dismissedKey(d);
		if (this.restoringKey === key) return;
		this.restoringKey = key;
		try {
			await invoke<void>("user_context_restore_dismissed", {
				subject: d.subject,
				statement: d.statement,
			});
			this.dismissed = (this.dismissed ?? []).filter((x) => dismissedKey(x) !== key);
			if (this.focus.kind === "dismissed" && dismissedKey(this.focus.item) === key) {
				this.focus = { kind: "none" };
			}
			this.dismissedError = null;
		} catch (error) {
			this.dismissedError = humanizeError(error);
		} finally {
			this.restoringKey = null;
		}
	}
}
