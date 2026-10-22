// Authored-context topics (IMPLEMENTATION.md §8 #9), shared by Context's topic
// chips and ContextSide's "What the dossier is built from" counts.
import type { AuthoredContext } from "$lib/types/recording";

/** The fixed topics, each on a category swatch: Work · Preferences · People · Goals, or none. */
export const TOPICS: { name: string; color: string }[] = [
  { name: "Work", color: "var(--cat-creating)" },
  { name: "Preferences", color: "var(--cat-organizing)" },
  { name: "People", color: "var(--cat-communication)" },
  { name: "Goals", color: "var(--cat-learning)" },
];
const OTHER_COLOR = "var(--chart-grey-3)";
export const topicColor = (t: string | null) => TOPICS.find((x) => x.name === t)?.color ?? OTHER_COLOR;

/**
 * Authored statements counted per topic. Statements with no topic, or a topic
 * outside `TOPICS` (rows written before the backend validated topics), land in
 * a trailing "Other" row — shown only when non-empty — so every statement is
 * counted somewhere.
 */
export function topicCounts(authored: Pick<AuthoredContext, "topic">[]): { name: string; color: string; count: number }[] {
  const rows = TOPICS.map((t) => ({ ...t, count: authored.filter((s) => s.topic === t.name).length }));
  const other = authored.length - rows.reduce((sum, r) => sum + r.count, 0);
  return other ? [...rows, { name: "Other", color: OTHER_COLOR, count: other }] : rows;
}
