/** Relative stamps for the Context destination ("added 3d ago", "dismissed 2w
 *  ago"). Carries the trailing word itself, unlike the rail's single-token
 *  `relativeTime` in `conversationStore.svelte.ts` — the rows here read as a
 *  sentence, the rail's stamps have to stay narrow. */
export function contextAgo(ms: number): string {
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
