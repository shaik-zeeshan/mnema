// Render-verification harness for direction 02 (Studio Shell).
//
// The app is verified by LOOKING at it, never by grepping class names — grepping
// gave three false verdicts on this codebase before. This drives the vite dev
// server in chromium with a stubbed Tauri IPC and writes one PNG per
// surface × theme × size.
//
// Usage: node scripts/shot-studio-shell.mjs <port> <outDir>
//
// ponytail: one flat script, no config file, no fixture framework. The stub map
// below is the whole contract — add a command to it only when a surface
// actually blanks without it.

import { mkdir } from "node:fs/promises";
import { chromium } from "playwright";

const port = process.argv[2] ?? "1420";
const outDir = process.argv[3] ?? "/tmp/ss-verify";
const base = `http://localhost:${port}`;

// The stub's only hard rule: NEVER return bare `null` for an unmocked command.
// A null return freezes the Svelte tree and every screenshot after it is blank.
const STUB = `
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: async () => {} };
const now = Date.now();
const day = 86400000;
const hour = 3600000;
const dayStart = (() => { const d = new Date(); d.setHours(0,0,0,0); return d.getTime(); })();
const at = (h, m) => dayStart + h * hour + m * 60000;
const facts = {
  capturePath: "/Users/you/Movies/Mnema",
  diskFreeBytes: 214_000_000_000,
  totalRamBytes: 16_000_000_000,
  measuredBytesPerDay: 1_400_000_000,
  measuredDays: 7,
  screenFrameRate: 2,
  ocrBacklog: 128,
  transcriptionBacklog: 3,
  semanticVectorCount: 0,
  semanticPendingCount: 0,
  semanticVectorBytes: 768,
  databaseBytes: 940_000_000,
};
// list_day_coverage returns capture_types::DayCoverage — day, coveredMs, hours.
// An earlier version of this stub invented dayStartMs/capturedMs, which every
// consumer silently read as "no capture"; keep this shape honest.
// (No backticks in this block — it lives inside the STUB template literal.)
const coverage = Array.from({ length: 7 }, (_, i) => ({
  day: new Date(now - (6 - i) * day).toISOString().slice(0, 10),
  coveredMs: [3.1, 6.4, 5.2, 7.8, 2.0, 0, 6.7][i] * hour,
  hours: [3.1, 6.4, 5.2, 7.8, 2.0, 0, 6.7][i],
}));
const moments = Array.from({ length: 6 }, (_, i) => ({
  frameId: 100 + i,
  filePath: "/tmp/none.png",
  capturedAtMs: now - i * hour,
  activityId: 10 + i,
  title: ["Reviewing the capture pipeline", "Slack — #design", "Figma — Studio Shell", "Terminal", "Docs — ADR 0052", "Safari — pricing"][i],
  focus: i % 2 === 0 ? "deep" : "shallow",
  activityStartedAtMs: now - i * hour - 1800000,
  activityEndedAtMs: now - i * hour,
  durationMs: 1800000,
}));

// ── Context (page 10): authored lines + the dismissed archive ──────────────
const authored = [
  { id: 1, text: "I run this repo with Bun, never pnpm — if you suggest a pnpm command it is wrong.", topic: "tooling", createdAtMs: now - 2 * hour, updatedAtMs: now - 2 * hour },
  { id: 2, text: "I'm the sole developer on Mnema — a local-first macOS app that records the screen so it can be searched later.", topic: "role", createdAtMs: now - 20 * day, updatedAtMs: now - 3 * day },
  { id: 3, text: "I prefer to work by shipping the smallest version that works and questioning it in the same breath.", topic: "how I work", createdAtMs: now - 7 * day, updatedAtMs: now - 7 * day },
  { id: 4, text: "Goal: ship the licensing flow and the redesign before the end of the quarter.", topic: "goal", createdAtMs: now - 14 * day, updatedAtMs: now - 14 * day },
  { id: 5, text: "I care about the app being honest about what it costs — disk, battery, and what leaves the machine.", topic: null, createdAtMs: now - 21 * day, updatedAtMs: now - 21 * day },
];
const dismissed = [
  { subject: "Diarisation", statement: "You are evaluating sortformer as a replacement for speakrs.", dismissedAtMs: now - 21 * day },
  { subject: "Working hours", statement: "You work primarily in the evenings.", dismissedAtMs: now - 35 * day },
  { subject: "Semantic search", statement: "Semantic search is planned for the next release.", dismissedAtMs: now - 42 * day },
];

// ── Subjects (page 09): twelve subjects across all four conviction tiers ───
// [subject, [ [statement, confidence, historyPoints, pinned, faded, agoMs] ]]
const SPEC = [
  ["Mnema licensing", [
    ["Polar keys the webhook HMAC on the raw whsec_ string, not the decoded key.", 0.86, [0.42,0.5,0.58,0.71,0.78,0.82,0.86], true, false, 3*hour],
    ["Polar is the merchant of record; the app never sees a card.", 0.74, [0.72,0.73,0.74,0.74,0.74], false, false, 2*day],
    ["Licence verification is offline Ed25519 — no server call to open the app.", 0.69, [0.55,0.58,0.62,0.66,0.69,0.69], false, false, day],
    ["CRL caching is still undecided.", 0.12, [0.38,0.3,0.12], false, true, 40*day],
  ]],
  ["Ask AI streaming", [
    ["The backend owns the render-ready turn; both doors only render.", 0.79, [0.76,0.77,0.78,0.78,0.79], false, false, 2*day],
    ["Answer blocks are parsed server-side, never in the frontend.", 0.71, [0.7,0.7,0.71,0.71], false, false, 3*day],
  ]],
  ["Bun, never pnpm", [
    ["Every repo command in this workspace runs through Bun.", 0.72, [0.71,0.72,0.72,0.72,0.72,0.72], false, false, 5*day],
  ]],
  ["Q3 launch checklist", [
    ["The Notion doc is revisited first thing most mornings.", 0.58, [0.31,0.36,0.42,0.48,0.53,0.58], false, false, 6*hour],
    ["Release blockers are triaged in the Monday launch sync.", 0.45, [0.3,0.36,0.41,0.45], false, false, 9*hour],
  ]],
  ["Deepgram streaming", [
    ["Read the WebSocket docs on Friday; no decision has followed.", 0.41, [0.62,0.58,0.52,0.47,0.41], false, false, 4*day],
  ]],
  ["OpenBLAS build chain", [
    ["Static linking is deliberate — the dylib was the launch crash.", 0.47, [0.46,0.47,0.47,0.46,0.47], false, false, 7*day],
  ]],
  ["System audio taps", [
    ["Core Audio process taps replaced the ScreenCaptureKit audio path.", 0.66, [0.4,0.48,0.55,0.6,0.66], false, false, 8*hour],
  ]],
  ["Speaker diarisation", [
    ["speakrs is the sole on-device provider now.", 0.61, [0.6,0.61,0.61,0.6,0.61], false, false, 3*day],
  ]],
  ["Onboarding rework", [
    ["Only two gates are hard; everything else is settings, not a plan.", 0.44, [0.3,0.34,0.39,0.44], false, false, 12*hour],
  ]],
  ["Retention policy", [
    ["Delete Recent Capture drops whole overlapping segments.", 0.39, [0.38,0.39,0.39], false, false, 6*day],
  ]],
  ["Speaker enrolment friction", [
    ["You have opened the voiceprint recorder twice and not finished it.", 0.22, [0.16,0.18,0.2,0.21,0.22], false, false, 9*hour],
  ]],
  ["Sortformer diarisation", [
    ["Considered as a speakrs alternative; not looked at since June.", 0.11, [0.34,0.28,0.22,0.17,0.13,0.11], false, true, 49*day],
  ]],
];

// ── Journal (page 08): one honest day ──────────────────────────────────────
// The morning's pixels are gone (footage expired), a real away gap at lunch,
// the derivation watermark at 15:30 so the live edge is deriving, and one
// activity carries cited spoken evidence.
const COVERED_UNTIL = at(15, 30);
const frames = [];
let fid = 1000;
for (let t = at(11, 0); t <= at(16, 40); t += 20000) {
  if (t >= at(12, 10) && t <= at(12, 40)) continue; // away — no capture
  frames.push({ id: fid++, capturedAt: new Date(t).toISOString(), filePath: "/frames/f.png" });
}
const fidAt = (h, m) => {
  const t = at(h, m);
  let best = frames[0];
  for (const f of frames) {
    if (Math.abs(Date.parse(f.capturedAt) - t) < Math.abs(Date.parse(best.capturedAt) - t)) best = f;
  }
  return best.id;
};
const ev = (type, id, headline, ms) => ({ subjectType: type, subjectId: id, capturedAtMs: ms ?? null, isHeadline: headline });
const activities = [
  { id: 1, title: "Chasing the Polar webhook signature mismatch", category: "creating", focus: "deep",
    startedAtMs: at(9, 14), endedAtMs: at(10, 26),
    summary: "You went back and forth between the Polar docs and webhook.rs — the HMAC was being computed over the decoded key rather than the raw whsec_ string.",
    evidence: [] },
  { id: 2, title: "Skimmed the Deepgram streaming docs", category: "learning", focus: "mixed",
    startedAtMs: at(10, 31), endedAtMs: at(10, 35), summary: "A quick read of the streaming endpoint's keep-alive rules.", evidence: [] },
  { id: 3, title: "Design review — the receipt overlay", category: "meetings", focus: "mixed",
    startedAtMs: at(11, 5), endedAtMs: at(11, 45),
    summary: "Three voices, mostly about whether the transcript belongs inside the receipt or beside it.",
    evidence: [ev("frame", fidAt(11, 20), true, at(11, 20)), ev("frame", fidAt(11, 33), false)] },
  { id: 4, title: "Launch sync — release blockers", category: "meetings", focus: "mixed",
    startedAtMs: at(12, 45), endedAtMs: at(13, 40),
    summary: "Five speakers. The webhook was still 400-ing in staging; Priya asked for delivery ids in the log so a failure can be traced without re-sending.",
    evidence: [ev("frame", fidAt(13, 2), true, at(13, 2)), ev("frame", fidAt(12, 52), false), ev("audio_segment", 900, false)] },
  { id: 5, title: "Filed the delivery-id ticket", category: "planning", focus: "mixed",
    startedAtMs: at(13, 50), endedAtMs: at(13, 53), summary: "Two fields, one ticket.", evidence: [] },
  { id: 6, title: "Parsing answer blocks server-side in answer_view.rs", category: "creating", focus: "deep",
    startedAtMs: at(14, 5), endedAtMs: at(15, 20),
    summary: "The fence parsing moved behind the backend so both doors render the same typed blocks.",
    evidence: [ev("frame", fidAt(14, 30), true, at(14, 30))] },
  { id: 7, title: "Reading the CRL cache ADR", category: "learning", focus: "deep",
    startedAtMs: at(15, 25), endedAtMs: at(15, 52),
    summary: "Checking whether the revocation list belongs in the licensing state or its own store.", evidence: [] },
].map((a) => ({ ...a, createdAtMs: a.endedAtMs }));
const digest = {
  rangeKind: "day", rangeStartMs: dayStart, rangeEndMs: dayStart + day,
  generatedAtMs: now - 22 * 60000,
  headline: "A licensing day that turned into a webhook day.",
  narrative: "The morning went to the Polar signature mismatch — the HMAC is applied to the raw whsec_ string. The launch sync turned it into a delivery-id question, and the afternoon has been answer_view.rs since.",
};

let cid = 1;
const conclusions = [];
for (const [subject, rows] of SPEC) {
  for (const [statement, confidence, history, pinned, faded, age] of rows) {
    const id = cid++;
    conclusions.push({
      id, subject, statement, confidence,
      status: faded ? "faded" : "active",
      pinned,
      formedAtMs: now - history.length * 6 * day,
      lastSupportedAtMs: now - age,
      updatedAtMs: now - age,
      evidence: activities.slice(0, (id % 3) + 1).map((a, i) => ({
        activityId: a.id,
        stance: i === 2 ? "contradict" : "support",
        activityTitle: a.title,
        activityStartedAtMs: a.startedAtMs,
      })),
      replacedStatement: id === 1 ? "Polar signs webhooks with the base64-decoded secret." : null,
      replacedAtMs: id === 1 ? now - 4 * day : null,
      _history: history,
    });
  }
}
// The context inspector's steering section matches authored topics to
// conclusion subjects — give it two real matches.
for (const [i, [subject, statement, confidence]] of [
  ["tooling", "Every repo command in this workspace runs through Bun", 0.72],
  ["role", "Licence verification is offline Ed25519", 0.69],
].entries()) {
  conclusions.push({
    id: 500 + i, subject, statement, confidence, status: "active", pinned: false,
    formedAtMs: now - 9 * day, lastSupportedAtMs: now - hour, updatedAtMs: now - hour,
    evidence: [], replacedStatement: null, replacedAtMs: null, _history: [confidence],
  });
}
const publicConclusions = conclusions.map(({ _history, ...c }) => c);
function subjectView(subject) {
  const cs = conclusions.filter((c) => c.subject === subject);
  return {
    subject,
    conclusions: cs.map(({ _history, ...c }) => c),
    trajectories: cs.map((c) => ({
      conclusionId: c.id,
      statement: c.statement,
      history: c._history.map((v, i) => ({
        confidence: v,
        snapshotAtMs: now - (c._history.length - 1 - i) * 3 * day,
      })),
    })),
  };
}

const MAP = {
  get_system_facts: facts,
  list_day_coverage: coverage,
  get_moments: moments,
  get_conversations: [],
  take_pending_license_deep_link: null,
  get_app_notifications: [],
  get_license_status: { licensed: { kind: "purchased", updatesUntilMs: now + 365 * day } },
  get_audio_transcription_model_status: { providers: [] },
  get_global_shortcut_registration_failures: [],
  get_ai_runtime_status: { enabled: true, available: true, reason: null },
  get_user_context_status: {
    engineAvailable: true, reason: null,
    activityCount: activities.length, conclusionCount: publicConclusions.length,
    lastDerivedAtMs: COVERED_UNTIL, coveredUntilMs: COVERED_UNTIL, backfilling: false,
    tokenUsage: { inputTokens: 120000, outputTokens: 8000, totalTokens: 128000, runCount: 42 },
    budgetTier: "balanced", lastDistillation: null,
    subjectCount: SPEC.length, dismissedCount: dismissed.length,
    skippedWindows24h: 0, localOffsetMinutes: new Date().getTimezoneOffset() * -1,
  },
  list_user_context_authored: authored,
  user_context_list_dismissed: dismissed,
  list_user_context_activities: activities,
  list_user_context_conclusions: publicConclusions,
  get_usage_charts: { timePerApp: [{ activeMs: 4.1 * hour }, { activeMs: 1.6 * hour }, { activeMs: 0.7 * hour }] },
  get_user_context_digest: digest,
  get_latest_user_context_digest: digest,
  regenerate_user_context_digest: digest,
  list_person_profiles: [],
};

window.__TAURI_INTERNALS__ = {
  transformCallback: (c) => c,
  convertFileSrc: (x) => x,
  invoke: async (cmd, args) => {
    if (cmd === "get_user_context_subject") return subjectView(args?.subject);
    if (cmd === "list_frame_summaries_in_range") {
      const req = args?.request ?? {};
      const s = Date.parse(req.capturedAtStart), e = Date.parse(req.capturedAtEnd);
      return frames.filter((f) => { const t = Date.parse(f.capturedAt); return t >= s && t < e; });
    }
    if (cmd in MAP) return MAP[cmd];
    if (cmd.startsWith("list_") || cmd.startsWith("get_recent")) return [];
    if (cmd.startsWith("has_pending")) return false;
    // Settings will not render unless an update echoes its domain back.
    if (cmd.startsWith("update_")) return { ...(args?.request ?? args ?? {}) };
    return {};
  },
};
`;

const SURFACES = [
	["timeline", "/"],
	["overview", "/overview"],
	["journal", "/journal"],
	["subjects", "/subjects"],
	["context", "/context"],
	["settings", "/settings"],
	["quick-recall", "/quick-recall"],
];
const SIZES = [
	["1100x720", 1100, 720],
	["800x600", 800, 600],
];

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch();

for (const theme of ["light", "dark"]) {
	for (const [w, width, height] of SIZES) {
		const ctx = await browser.newContext({ viewport: { width, height } });
		await ctx.addInitScript(STUB);
		// The runtime owns the theme decision (`data-theme` on <html>), so set it
		// the way the app does rather than leaning on prefers-color-scheme. The
		// theme store re-applies data-theme after its settings read, so PIN it with
		// a MutationObserver — a one-shot set silently loses to the store.
		await ctx.addInitScript(`
      const pin = ${JSON.stringify(theme)};
      document.addEventListener("DOMContentLoaded", () => {
        document.documentElement.dataset.theme = pin;
        new MutationObserver(() => {
          if (document.documentElement.dataset.theme !== pin) {
            document.documentElement.dataset.theme = pin;
          }
        }).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
      });
      localStorage.setItem("mnema.appearance", ${JSON.stringify(theme)});
    `);
		const page = await ctx.newPage();
		const errors = [];
		page.on("pageerror", (e) => errors.push(String(e)));

		for (const [name, path] of SURFACES) {
			try {
				await page.goto(`${base}${path}`, { waitUntil: "networkidle", timeout: 20000 });
			} catch {
				await page.goto(`${base}${path}`, { timeout: 20000 }).catch(() => {});
			}
			await page.waitForTimeout(1200);
			const file = `${outDir}/${name}-${theme}-${w}.png`;
			await page.screenshot({ path: file });
			console.log(file);
		}
		if (errors.length) console.log(`  page errors (${theme}/${w}):`, errors.slice(0, 4));
		await ctx.close();
	}
}

await browser.close();
