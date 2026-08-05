// Round-4 direction-05 render harness.
//
// The ONLY sanctioned way to verify this branch: screenshot the real app in a
// real browser and LOOK at the PNG. Grepping class names has produced false
// verdicts on this repo before.
//
//   bun x playwright install chromium          # once
//   nohup bun --cwd=apps/desktop run dev > /tmp/vite-05.log 2>&1 & disown
//   node scripts/rd4-shot.mjs /overview dark 1100x720 docs/redesign/round4/05-tactile-instruments/shots
//
// Args: <route> <theme: light|dark> <WxH> <outDir> [portFromViteLog]
// The port auto-bumps if 1420 is taken — read /tmp/vite-05.log for the real one
// and pass it via the PORT env var.

import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const [route = "/", theme = "dark", size = "1100x720", outDir = "/tmp/rd4-shots"] = process.argv.slice(2);
const port = process.env.PORT ?? "1420";
const [width, height] = size.split("x").map(Number);

// Tauri IPC stub. Rule: NEVER return bare `null` for an unmocked command — a
// null freezes the Svelte tree and every subsequent screenshot is a lie.
const initScript = `
// ── Timeline fixture (additive) ──────────────────────────────────────────────
// Enough shape for the rail to draw app bands, the lane to draw mic/sys bars,
// the jump menu to have days you can land on, and the drawer to open with a
// waveform + turns. Every number here is fixture data, never copy.
const TL_NOW = Date.now();
const TL_STEP = 30000;                       // one frame every 30 s
const TL_APPS = [
  ["com.microsoft.VSCode", "Code", "answer_view.rs — mnema", 34],
  ["com.apple.Safari", "Safari", "docs.polar.sh — webhook signatures", 22],
  ["com.tinyspeck.slackmacgap", "Slack", "#launch — 3 unread", 18],
  ["com.figma.Desktop", "Figma", "Round 4 — timeline", 12],
  ["md.obsidian", "Obsidian", "Mnema.md", 15],
  ["com.apple.Terminal", "Terminal", "cargo test -p app-infra", 9],
];
const TL_FRAMES = (() => {
  const out = [];
  let id = 4000;
  let t = TL_NOW - 2000;
  let a = 0;
  while (out.length < 260) {
    const [bundleId, appName, windowTitle, run] = TL_APPS[a % TL_APPS.length];
    for (let k = 0; k < run && out.length < 260; k++) {
      out.push({
        id: id--, sessionId: "sess-fixture", filePath: "/fixture/frame.png",
        capturedAt: new Date(t).toISOString(), width: 2560, height: 1440,
        appBundleId: bundleId, appName, windowTitle,
        url: null, ocrText: null, processorVersion: null, equivalenceHint: null,
        createdAt: new Date(t).toISOString(), updatedAt: new Date(t).toISOString(),
      });
      t -= TL_STEP;
    }
    a++;
  }
  return out;                                // newest first, as the backend returns
})();
// A CSS-free stand-in screenshot: a dark editor-ish SVG, so the stage has a real
// 16:9 image to float instead of a grey rectangle.
const TL_SHOT = "data:image/svg+xml;base64," + btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720">' +
  '<rect width="1280" height="720" fill="#15151d"/>' +
  '<rect width="70" height="720" fill="#0f0f16"/><rect x="70" width="250" height="720" fill="#15151d"/>' +
  '<rect x="320" width="960" height="46" fill="#0f0f16"/>' +
  Array.from({ length: 22 }, (_, i) =>
    '<rect x="' + (360 + (i % 3) * 24) + '" y="' + (76 + i * 27) + '" width="' +
    (160 + ((i * 97) % 520)) + '" height="10" rx="2" fill="' +
    ["#4c4c62", "#c08cf0", "#8fd694", "#6fb8f0", "#48584f"][i % 5] + '"/>').join("") +
  Array.from({ length: 12 }, (_, i) =>
    '<rect x="96" y="' + (80 + i * 34) + '" width="' + (90 + ((i * 53) % 150)) +
    '" height="9" rx="2" fill="#33334a"/>').join("") +
  '</svg>');
const TL_SEGS = [
  ["microphone", 1, -95, -62], ["systemAudio", 1, -95, -62],
  ["microphone", 2, -48, -30], ["systemAudio", 2, -140, -118],
  ["systemAudio", 3, -12, -2],
].map(([sourceKind, segmentIndex, fromMin, toMin], i) => ({
  id: 900 + i, sourceKind, sourceSessionId: "sess-fixture", segmentIndex,
  filePath: "/fixture/" + sourceKind + "-" + segmentIndex + ".m4a",
  startedAt: new Date(TL_NOW + fromMin * 60000).toISOString(),
  endedAt: new Date(TL_NOW + toMin * 60000).toISOString(),
  createdAt: new Date(TL_NOW).toISOString(), updatedAt: new Date(TL_NOW).toISOString(),
}));
const TL_COVERAGE = (() => {
  const out = [];
  for (let back = 0; back < 34; back++) {
    if (back === 4 || back === 9 || back === 10 || back === 17) continue;   // empty days
    const d = new Date(TL_NOW - back * 86400000);
    const key = d.getFullYear() + "-" + String(d.getMonth() + 1).padStart(2, "0") +
      "-" + String(d.getDate()).padStart(2, "0");
    const first = 8 + (back % 3);
    const last = first + 3 + ((back * 5) % 7);
    const hours = [];
    for (let h = first; h <= Math.min(22, last); h++) hours.push(h);
    out.push({ day: key, coveredMs: hours.length * 2400000, hours });
  }
  return out;
})();
// A real (silent) WAV so the drawer's <audio> loads instead of erroring.
const TL_WAV = (() => {
  const n = 8000, b = new Uint8Array(44 + n * 2), dv = new DataView(b.buffer);
  const put = (o, s) => { for (let i = 0; i < s.length; i++) b[o + i] = s.charCodeAt(i); };
  put(0, "RIFF"); dv.setUint32(4, 36 + n * 2, true); put(8, "WAVEfmt ");
  dv.setUint32(16, 16, true); dv.setUint16(20, 1, true); dv.setUint16(22, 1, true);
  dv.setUint32(24, 8000, true); dv.setUint32(28, 16000, true);
  dv.setUint16(32, 2, true); dv.setUint16(34, 16, true);
  put(36, "data"); dv.setUint32(40, n * 2, true);
  let s = ""; for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
  return btoa(s);
})();
const TL_TURNS = [
  [1, "Speaker 1", 6000, 21000, "The webhook is still 400-ing on the raw secret — I think we are hashing the decoded bytes."],
  [2, "Speaker 2", 22000, 38000, "That would explain the fixtures passing locally. I will check webhook.rs after this."],
  [3, "Speaker 3", 39000, 52000, "Can we get delivery ids into the log line while you are in there?"],
  [1, "Speaker 1", 53000, 74000, "Yes — and the retry count, otherwise the dashboard cannot tell a flake from a real failure."],
].map(([clusterId, speakerLabel, startMs, endMs, transcriptText], i) => ({
  id: 700 + i, audioSegmentId: 900, sessionId: "sess-fixture", clusterId,
  segmentClusterId: clusterId, providerClusterId: "p" + clusterId, speakerLabel,
  personId: null, personLinkAuto: false, suggestedPersonId: null,
  recognitionConfidence: null, recognitionScore: null,
  startMs, endMs, transcriptText, overlaps: false,
}));

window.__TAURI_INTERNALS__ = {
  invoke: async (cmd, args) => {
    const req = (args && (args.request ?? args.payload ?? args)) || {};
    if (cmd === "take_pending_license_deep_link") return null;          // the one legal null
    if (cmd === "list_frames") return req.beforeId ? [] : TL_FRAMES;
    if (cmd === "get_frame") return TL_FRAMES[0];
    if (cmd === "get_latest_frame_in_range") return TL_FRAMES[0];
    if (cmd === "get_frame_preview" || cmd === "get_scrub_preview")
      return { mimeType: "image/svg+xml", filePath: TL_SHOT, sourceKind: "original_frame", hasSecretRedactions: false, secretRedactionCount: 0 };
    if (cmd === "get_scrub_preview_availability") return [];
    if (cmd === "list_audio_segments") return TL_SEGS;
    if (cmd === "get_audio_segment") return TL_SEGS[0];
    if (cmd === "list_day_coverage") return TL_COVERAGE;
    if (cmd === "get_audio_segment_media") return { mimeType: "audio/wav", dataBase64: TL_WAV };
    if (cmd === "get_audio_segment_waveform_peaks")
      return Array.from({ length: req.bucketCount ?? 96 }, (_, i) =>
        0.12 + 0.78 * Math.abs(Math.sin(i / 3.7)) * (0.45 + 0.55 * Math.abs(Math.cos(i / 11))));
    if (cmd === "list_speaker_turns") return TL_TURNS;
    if (cmd === "get_app_notifications") return [];
    if (cmd === "get_license_status") return { licensed: { kind: "purchased", updatesUntilMs: Date.now() + 3.15e10 } };
    if (cmd === "get_audio_transcription_model_status") return { providers: [] };
    if (cmd.startsWith("update_")) return { ...req, domain: req.domain ?? args?.domain ?? "capture" };
    if (cmd.startsWith("list_")) return [];
    if (cmd.startsWith("has_pending")) return false;
    if (cmd.startsWith("search_")) return [];
    // Anything a caller iterates or spreads must be an array, not {}.
    if (/failures|_list$|models|providers|apps|entries|items|results|conversations|moments|coverage|charts|languages|devices|sources|bindings$/.test(cmd)) return [];
    return {};
  },
  transformCallback: (c) => c,
  convertFileSrc: (x) => x,
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: async () => {} };
`;

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 2 });
await page.addInitScript(initScript);

const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

await page.goto(`http://localhost:${port}${route}`, { waitUntil: "networkidle" });
// data-theme is set pre-navigation, but SvelteKit's theme init may overwrite it.
await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
await page.waitForTimeout(900);

// The stub's `update_*` echo is not a real settings object, so the autosave
// path raises "Couldn't save …" toasts that are artifacts of the harness, not
// of the UI under test — and they sit on top of the bottom-right of every
// screenshot. Hidden by default; set SHOW_TOASTS=1 when the toast IS the thing
// you are verifying.
if (process.env.SHOW_TOASTS !== "1") {
  await page.addStyleTag({ content: ".toast-stack { display: none !important; }" });
}

// Opt-in interaction before the shot, for states that only exist after a click
// (a menu open, a drawer open). CLICK is a selector; SUFFIX names the PNG so
// two states of one route don't overwrite each other.
if (process.env.CLICK) {
  for (const sel of process.env.CLICK.split("|")) {
    await page.locator(sel.trim()).first().click({ timeout: 5000 }).catch((e) => errors.push(`CLICK ${sel}: ${e.message}`));
    await page.waitForTimeout(500);
  }
  await page.waitForTimeout(600);
}

const name = `${route.replace(/\W+/g, "_") || "root"}${process.env.SUFFIX ? "-" + process.env.SUFFIX : ""}-${theme}-${size}.png`;
const out = path.join(outDir, name);
await page.screenshot({ path: out, fullPage: false });
await browser.close();

console.log(out);
if (errors.length) console.log("PAGE ERRORS:\n" + errors.slice(0, 8).join("\n"));
