// Small display helpers for the Chat surface (list stamps, model picker chips,
// the per-answer readouts). Pure, so they're tested beside this file.
import type { AiProviderConfig } from "$lib/types/recording";
import type { ConversationSummary } from "$lib/insights/conversation";

const DAY_MS = 86_400_000;

/** Where a provider instance runs: Ollama / Llamafile, or any endpoint on this
 *  Mac, is "local"; everything else is "cloud". */
export function providerWhere(provider: AiProviderConfig | null | undefined): "cloud" | "local" {
  if (!provider) return "cloud";
  if (provider.kind === "ollama" || provider.kind === "llamafile") return "local";
  return /\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/i.test(provider.baseUrl ?? "") ? "local" : "cloud";
}

/** How a provider is reached, for the picker's group header ("your API key",
 *  "localhost:11434", …). */
export function providerVia(provider: AiProviderConfig | null | undefined): string {
  if (!provider) return "";
  if (provider.kind === "chatgpt") return "your ChatGPT sign-in";
  const url = provider.baseUrl?.trim() ?? "";
  if (url) {
    try {
      return new URL(url).host || url;
    } catch {
      return url;
    }
  }
  return provider.kind === "ollama" || provider.kind === "llamafile" ? "on this Mac" : "your API key";
}

/** A chat-list row stamp: clock time today and yesterday, the weekday within
 *  the week, else a short date ("Sep 30"; the year only when it differs). */
export function chatWhen(ms: number, now: number = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const d = new Date(ms);
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const start = today.getTime();
  if (ms >= start - DAY_MS) return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (ms >= start - 6 * DAY_MS) return d.toLocaleDateString(undefined, { weekday: "short" });
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(d.getFullYear() === today.getFullYear() ? {} : { year: "numeric" }),
  });
}

/** "812" / "6.2k" / "14k" / "1.2M". */
export function formatTokenCount(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(tokens >= 10_000 ? 0 : 1)}k`;
  return `${tokens}`;
}

/** The new-chat greeting for a local hour (0–23). */
export function greeting(hour: number): string {
  if (hour >= 5 && hour < 12) return "Good morning.";
  if (hour >= 12 && hour < 18) return "Good afternoon.";
  return "Good evening.";
}

/** The answer's steps line: "4 steps · 3.4s" (the duration only for an answer
 *  timed in this session — step timing isn't persisted). Null with no steps. */
export function stepsLine(steps: number, elapsedMs: number | null): string | null {
  if (steps <= 0) return null;
  const head = `${steps} step${steps === 1 ? "" : "s"}`;
  return elapsedMs !== null && elapsedMs > 0 ? `${head} · ${(elapsedMs / 1000).toFixed(1)}s` : head;
}

// ── Chat-list grouping ───────────────────────────────────────────────────────
// Pinned chats first (only there, never also under their date), then quiet date
// headers from each chat's last activity (`updatedAtMs`, the list's sort key):
// Today / Yesterday / This week (the rest of the last 7 calendar days) / earlier
// months ("May 2026"). Buckets keep first-seen order, so the backend's sort
// order holds within each group and search never duplicates a header.
export interface HistoryGroup {
  label: string;
  items: ConversationSummary[];
}

function historyGroupLabel(ms: number, todayStartMs: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "Earlier";
  if (ms >= todayStartMs) return "Today";
  if (ms >= todayStartMs - DAY_MS) return "Yesterday";
  if (ms >= todayStartMs - 6 * DAY_MS) return "This week";
  return new Date(ms).toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

export function groupHistory(conversations: ConversationSummary[], now: number = Date.now()): HistoryGroup[] {
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const todayStartMs = today.getTime();
  const groups: HistoryGroup[] = [];
  const byLabel = new Map<string, HistoryGroup>();
  for (const c of conversations) {
    const label = c.pinned ? "Pinned" : historyGroupLabel(c.updatedAtMs, todayStartMs);
    let group = byLabel.get(label);
    if (group === undefined) {
      group = { label, items: [] };
      byLabel.set(label, group);
      if (label === "Pinned") groups.unshift(group);
      else groups.push(group);
    }
    group.items.push(c);
  }
  return groups;
}
