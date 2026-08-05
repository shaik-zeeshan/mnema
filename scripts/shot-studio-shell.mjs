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
  coveredMs: [3.1, 6.4, 5.2, 7.8, 2.0, 0, 6.7][i] * 3600000,
  hours: [3.1, 6.4, 5.2, 7.8, 2.0, 0, 6.7][i],
}));
const moments = Array.from({ length: 6 }, (_, i) => ({
  frameId: 100 + i,
  filePath: "/tmp/none.png",
  capturedAtMs: now - i * 3600000,
  activityId: 10 + i,
  title: ["Reviewing the capture pipeline", "Slack — #design", "Figma — Studio Shell", "Terminal", "Docs — ADR 0052", "Safari — pricing"][i],
  focus: i % 2 === 0 ? "deep" : "shallow",
  activityStartedAtMs: now - i * 3600000 - 1800000,
  activityEndedAtMs: now - i * 3600000,
  durationMs: 1800000,
}));

const MAP = {
  get_system_facts: facts,
  list_day_coverage: coverage,
  get_moments: moments,
  get_conversations: [],
  take_pending_license_deep_link: null,
  get_app_notifications: [],
  get_license_status: { licensed: { kind: "purchased", updatesUntilMs: now + 365 * day } },
  get_audio_transcription_model_status: { providers: [] },
};

window.__TAURI_INTERNALS__ = {
  transformCallback: (c) => c,
  convertFileSrc: (x) => x,
  invoke: async (cmd, args) => {
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
		// the way the app does rather than leaning on prefers-color-scheme.
		await ctx.addInitScript(`
      document.addEventListener("DOMContentLoaded", () => {
        document.documentElement.dataset.theme = ${JSON.stringify(theme)};
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
			await page.evaluate((t) => {
				document.documentElement.dataset.theme = t;
			}, theme);
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
