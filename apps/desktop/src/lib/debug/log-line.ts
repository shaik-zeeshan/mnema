// One app-log line, parsed for the Logs tab. Pure, so `bun test` can import it
// (a runes module can't be).
//
// rust.log lines look like
//   `[2026-10-13][15:01:26][mnema_lib::semantic_search_worker][INFO] message`
// Anything else (a panic backtrace, a multi-line message) is a continuation line.

export interface LogLine {
	/** Local wall clock, `HH:MM:SS` (+ `.mmm` when the source has millis). */
	time: string;
	/** `INFO` / `WARN` / `ERROR` / `DEBUG` / `TRACE`. */
	level: string;
	/** The real module path, e.g. `capture_system_audio::rebuild`. */
	target: string;
	/** The bold lead: the message's own tag (`system-audio-tap:`, `[app-infra]`), else the target's last segment. */
	prefix: string;
	/** The message without its tag. */
	message: string;
}

const RUST_LINE =
	/^\[(\d{4})-(\d{2})-(\d{2})\]\[(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,9}))?\]\[([^\]]*)\]\[([A-Z]+)\] ?(.*)$/;
/** A leading `tag:` or `[tag]` — the convention the capture crates and app-infra log with. */
const MESSAGE_TAG = /^(\[[\w:.-]+\]|[\w.-]+:)\s+(.*)$/;

const pad = (n: number, w = 2) => String(n).padStart(w, "0");

function localClock(date: Date, millis: boolean): string {
	const hms = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
	return millis ? `${hms}.${pad(date.getMilliseconds(), 3)}` : hms;
}

function split(text: string, fallback: string): { prefix: string; message: string } {
	const tag = MESSAGE_TAG.exec(text);
	return tag ? { prefix: tag[1], message: tag[2] } : { prefix: fallback, message: text };
}

/** Parse one raw line; null for a continuation line (render it message-only). */
export function parseLogLine(raw: string): LogLine | null {
	const rust = RUST_LINE.exec(raw);
	if (rust) {
		const [, y, mo, d, h, mi, s, frac, target, level, text] = rust;
		// rust.log stamps UTC (tauri-plugin-log's default timezone strategy).
		const ms = frac ? Number(frac.slice(0, 3).padEnd(3, "0")) : 0;
		const date = new Date(Date.UTC(+y, +mo - 1, +d, +h, +mi, +s, ms));
		const lastSegment = target.split("::").pop() ?? target;
		return { time: localClock(date, frac != null), level, target, ...split(text, lastSegment) };
	}
	return null;
}

/** A parsed head line plus the continuation lines under it; `head` is null for orphans at the top of the tail. */
export interface LogEntry {
	head: LogLine | null;
	/** Continuation lines, raw. */
	more: string[];
	/** Every raw line of the entry, newline-joined — what a click copies and the filters test. */
	raw: string;
}

/** Group raw lines (oldest first) so a backtrace stays under the line that started it. */
export function groupLogLines(lines: string[]): LogEntry[] {
	const entries: LogEntry[] = [];
	for (const raw of lines) {
		const head = parseLogLine(raw);
		const last = entries.at(-1);
		if (head == null && last) {
			last.more.push(raw);
			last.raw += `\n${raw}`;
		} else {
			entries.push(head ? { head, more: [], raw } : { head: null, more: [raw], raw });
		}
	}
	return entries;
}

export type LogLevelFilter = "all" | "warn" | "error";

/** Warnings = WARN + ERROR; Errors = ERROR. A headless entry only shows under All. */
export function levelMatches(level: string | null, filter: LogLevelFilter): boolean {
	if (filter === "all") return true;
	if (filter === "error") return level === "ERROR";
	return level === "WARN" || level === "ERROR";
}
