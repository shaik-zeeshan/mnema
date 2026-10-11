// Natural language → search operators for the Quick Recall field (recall.html
// "Natural language → tokens"). Recognised phrases are tinted in place while
// you type and turn into pills after a pause; each pill serialises to the
// operators the backend parser already knows (`filter-chips.ts`). Weekdays
// resolve to an ISO day HERE, because `search/dates.rs` only knows
// today / yesterday / Nd / ISO / this-week / last-week.
//
// ponytail: regex rules over a fixed vocabulary plus the captured-app names;
// a phrase inside double quotes is still read as a phrase (no quote masking).

export type PartOfDay = "morning" | "afternoon" | "evening";
export type SourceValue = "mic" | "system" | "screen";

export type Pill =
  | { kind: "app"; app: string }
  // `value` is the `date:` operator value; null = a part of day on its own.
  | { kind: "date"; value: string | null; label: string; part: PartOfDay | null }
  | { kind: "source"; source: SourceValue };

export type NaturalSpan = { start: number; end: number; pill: Pill };

export const PILL_ORDER: Pill["kind"][] = ["app", "date", "source"];

// `hours:H-H`, read in local time by the backend (QR-B1).
export const PART_HOURS: Record<PartOfDay, [number, number]> = {
  morning: [5, 12],
  afternoon: [12, 18],
  evening: [18, 24],
};

const WEEKDAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
const NUMBERS: Record<string, number> = { one: 1, two: 2, three: 3, four: 4, five: 5, six: 6 };
const SOURCES: Record<string, SourceValue> = {
  mic: "mic",
  microphone: "mic",
  system: "system",
  system_audio: "system",
  screen: "screen",
};
const PERIODS: Record<string, string> = {
  "this-week": "This week",
  "last-week": "Last week",
  "this-month": "This month",
  "last-month": "Last month",
};

const pad = (n: number) => String(n).padStart(2, "0");
const isoDay = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const daysBefore = (now: Date, n: number) =>
  new Date(now.getFullYear(), now.getMonth(), now.getDate() - n);
const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// A one-day pill: today / yesterday keep their named operator; older days
// carry the ISO day the backend accepts.
function oneDay(now: Date, back: number, part: PartOfDay | null = null): Pill {
  if (back === 0) return { kind: "date", value: "today", label: "Today", part };
  if (back === 1) return { kind: "date", value: "yesterday", label: "Yesterday", part };
  const d = daysBefore(now, back);
  const label = d.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
  return { kind: "date", value: isoDay(d), label: label.replace(",", ""), part };
}

const asPart = (s: string | undefined) => (s ? (s.toLowerCase() as PartOfDay) : null);

// Resolve a typed `date:` value to a pill, or null when the backend would
// reject it (it then stays text and the backend's parse error underlines it).
function dateOperator(now: Date, raw: string): Pill | null {
  const v = raw.toLowerCase();
  if (v === "today") return oneDay(now, 0);
  if (v === "yesterday") return oneDay(now, 1);
  if (PERIODS[v]) return { kind: "date", value: v, label: PERIODS[v], part: null };
  const m = v.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return null;
  const d = new Date(+m[1], +m[2] - 1, +m[3]);
  if (isNaN(d.getTime()) || isoDay(d) !== v) return null;
  const today = daysBefore(now, 0);
  return oneDay(now, Math.round((today.getTime() - d.getTime()) / 86_400_000));
}

type Rule = [RegExp, (m: RegExpMatchArray) => Pill | null];

function rules(now: Date, apps: readonly string[]): Rule[] {
  const findApp = (v: string) => {
    const low = v.toLowerCase();
    return (
      apps.find((a) => a.toLowerCase() === low) ??
      apps.find((a) => a.toLowerCase().startsWith(low)) ??
      null
    );
  };
  const out: Rule[] = [
    [
      /\b(app|source|date|hours):("[^"]+"|\S+)/gi,
      (m) => {
        const op = m[1].toLowerCase();
        const v = m[2].replace(/"/g, "");
        if (op === "app") {
          const app = findApp(v);
          return app ? { kind: "app", app } : null;
        }
        if (op === "source") {
          const source = SOURCES[v.toLowerCase()];
          return source ? { kind: "source", source } : null;
        }
        if (op === "hours") {
          const part = (Object.keys(PART_HOURS) as PartOfDay[]).find(
            (p) => PART_HOURS[p].join("-") === v,
          );
          return part ? { kind: "date", value: null, label: "", part } : null;
        }
        return dateOperator(now, v);
      },
    ],
    [
      /\b(?:(?:on|last)\s+)?(sunday|monday|tuesday|wednesday|thursday|friday|saturday)(?:\s+(morning|afternoon|evening))?\b/gi,
      (m) => {
        const back = (now.getDay() - WEEKDAYS.indexOf(m[1].toLowerCase()) + 7) % 7 || 7;
        return oneDay(now, back, asPart(m[2]));
      },
    ],
    [
      /\b(?:this\s+(morning|afternoon|evening)|(tonight))\b/gi,
      (m) => oneDay(now, 0, asPart(m[1]) ?? "evening"),
    ],
    [
      /\b(today|yesterday)(?:\s+(morning|afternoon|evening))?\b/gi,
      (m) => oneDay(now, m[1].toLowerCase() === "today" ? 0 : 1, asPart(m[2])),
    ],
    [
      /\b(this|last)\s+week\b/gi,
      (m) => {
        const value = `${m[1].toLowerCase()}-week`;
        return { kind: "date", value, label: PERIODS[value], part: null };
      },
    ],
    [
      /\b(\d+|one|two|three|four|five|six)\s+days?\s+ago\b/gi,
      (m) => oneDay(now, NUMBERS[m[1].toLowerCase()] ?? Number(m[1])),
    ],
  ];
  // A bare app name collides with ordinary words ("Notes", "Mail", "Preview"),
  // so a captured app only counts after in / on / from / using.
  const names = [...apps].filter((a) => a.trim()).sort((a, b) => b.length - a.length);
  if (names.length > 0) {
    out.push([
      new RegExp(`\\b(?:in|on|from|using)\\s+(${names.map(escapeRe).join("|")})(?![\\w-])`, "gi"),
      (m) => ({ kind: "app", app: findApp(m[1]) ?? m[1] }),
    ]);
  }
  out.push(
    [/\b(?:on|in|during)\s+(?:a\s+|the\s+)?(?:call|meeting)s?\b/gi, () => ({ kind: "source", source: "mic" })],
    [/\bon\s+(?:my\s+|the\s+)?screen\b/gi, () => ({ kind: "source", source: "screen" })],
    [/\b(?:in|on)\s+a\s+(?:podcast|video)\b/gi, () => ({ kind: "source", source: "system" })],
    [
      /\b(?:in\s+the\s+)?(morning|afternoon|evening)\b/gi,
      (m) => ({ kind: "date", value: null, label: "", part: asPart(m[1]) }),
    ],
  );
  return out;
}

// The recognised phrases in `text` (earlier rules win an overlap) and the
// free text left once they are lifted out.
export function parseNatural(
  text: string,
  now: Date,
  apps: readonly string[] = [],
): { spans: NaturalSpan[]; residual: string } {
  const spans: NaturalSpan[] = [];
  for (const [re, make] of rules(now, apps)) {
    for (const m of text.matchAll(re)) {
      const start = m.index ?? 0;
      const end = start + m[0].length;
      if (spans.some((s) => start < s.end && end > s.start)) continue;
      const pill = make(m);
      if (pill) spans.push({ start, end, pill });
    }
  }
  spans.sort((a, b) => a.start - b.start);
  let residual = "";
  let i = 0;
  for (const s of spans) {
    residual += text.slice(i, s.start);
    i = s.end;
  }
  residual = (residual + text.slice(i))
    .replace(/\s{2,}/g, " ")
    .replace(/\s+([?.!,])/g, "$1")
    .trim();
  return { spans, residual };
}

// One pill per kind, in PILL_ORDER. A part of day on its own narrows the date
// pill already there; a new date keeps a part-only pill's part.
export function mergePills(base: readonly Pill[], add: readonly Pill[]): Pill[] {
  const byKind = new Map(base.map((p) => [p.kind, p] as const));
  for (const p of add) {
    const prev = byKind.get(p.kind);
    if (p.kind === "date" && prev?.kind === "date") {
      if (p.value === null) {
        byKind.set("date", { ...prev, part: p.part });
        continue;
      }
      if (prev.value === null && p.part === null) {
        byKind.set("date", { ...p, part: prev.part });
        continue;
      }
    }
    byKind.set(p.kind, p);
  }
  return PILL_ORDER.flatMap((k) => byKind.get(k) ?? []);
}

export type MirrorSegment = { text: string; kind: "plain" | "live" | "bad" };

// The field's mirror: plain text, recognised phrases (tinted), and the token
// the backend rejected (`bad`, wavy underline). Spans never overlap `bad`
// because a rejected operator value is never recognised as a phrase.
export function mirrorSegments(
  text: string,
  spans: readonly NaturalSpan[],
  badToken: string | null,
): MirrorSegment[] {
  const marks: { start: number; end: number; kind: "live" | "bad" }[] = spans.map((s) => ({
    start: s.start,
    end: s.end,
    kind: "live",
  }));
  const at = badToken ? text.indexOf(badToken) : -1;
  if (badToken && at >= 0 && !marks.some((m) => at < m.end && at + badToken.length > m.start)) {
    marks.push({ start: at, end: at + badToken.length, kind: "bad" });
    marks.sort((a, b) => a.start - b.start);
  }
  const out: MirrorSegment[] = [];
  let i = 0;
  for (const m of marks) {
    if (m.start > i) out.push({ text: text.slice(i, m.start), kind: "plain" });
    out.push({ text: text.slice(m.start, m.end), kind: m.kind });
    i = m.end;
  }
  if (i < text.length) out.push({ text: text.slice(i), kind: "plain" });
  return out;
}
