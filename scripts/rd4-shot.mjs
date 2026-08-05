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
// 2 s apart = the shipping default rate. It must stay under the app-run split
// threshold (timelineGapMs, 10 s floor) or every frame becomes its own run and
// the rail draws 149 one-frame bands instead of app bands.
const TL_STEP = 2000;
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
// Offsets are MINUTES before now. They must land inside the RAIL'S VISIBLE
// SPAN (~2 min at 8 px/frame), not merely inside the loaded window, or the
// lane looks empty; the last one straddles now so the active frame has an
// audio moment (that is what the P shortcut opens).
const TL_SEGS = [
  ["microphone", 1, -2.3, -1.6], ["systemAudio", 1, -2.3, -1.6],
  ["microphone", 2, -1.1, -0.4], ["systemAudio", 2, -3.0, -2.6],
  ["systemAudio", 3, -0.9, 0.05],
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
// Month summaries feed the jump menu's hour list; without them every previewed
// day reads "No frames on this day". One id per covered hour is enough.
const TL_SUMMARIES = (range) => {
  const s = new Date(range.capturedAtStart).getTime();
  const e = new Date(range.capturedAtEnd).getTime();
  const out = [];
  let id = 1;
  for (const day of TL_COVERAGE) {
    for (const h of day.hours) {
      const [y, m, d] = day.day.split("-").map(Number);
      const at = new Date(y, m - 1, d, h, 12).getTime();
      if (at < s || at > e) continue;
      for (let k = 0; k < 3 + (h % 5); k++) out.push({ id: id++, capturedAt: new Date(at + k * 60000).toISOString() });
    }
  }
  return out;
};
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

// ── Overview bento fixture (additive) ───────────────────────────────────────
// Enough for the tiles to have something to read: headline frames, the day's
// conversations, a digest that carries its own open-thread sentence (G11), the
// context counts, and REAL-SHAPED system facts so the day-budget gauge can
// compute a denominator. Set EMPTY=1 to shoot the fresh-machine face instead —
// every read below then answers empty, exactly as day one does.
const OV_EMPTY = ${process.env.EMPTY === "1"};
// SEM=on  → semantic search enabled with a part-built index (the coverage meter)
// SEM=off → disabled but with a real un-indexed count (the price-before-enable
//           ghost gauge, G10). Unset keeps the shipping default: both counts
//           null, which is what prod ships (zero embeddings).
const SEM = "${process.env.SEM ?? ""}";
const SEM_FACTS = SEM === "on"
  ? { semanticVectorCount: 412000, semanticPendingCount: 248000 }
  : SEM === "off"
    ? { semanticVectorCount: 0, semanticPendingCount: 1240000 }
    : {};
const OV_SHOT = (fill, ink) => "data:image/svg+xml;base64," + btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="300">' +
  '<rect width="480" height="300" fill="' + fill + '"/>' +
  Array.from({ length: 10 }, (_, i) =>
    '<rect x="' + (40 + (i % 2) * 18) + '" y="' + (36 + i * 24) + '" width="' +
    (110 + ((i * 71) % 260)) + '" height="8" rx="2" fill="' + ink + '"/>').join("") +
  '</svg>');
const OV_MOMENTS = [
  ["Obsidian", "#1d1d24", "#4c4c62", 9.2], ["Figma", "#2c2c33", "#a259ff", 10.7],
  ["Zoom", "#16161c", "#5b8def", 13.0], ["Terminal", "#0c0e0d", "#4ade80", 13.8],
  ["Code", "#1b1b24", "#6fb8f0", 14.5],
].map(([title, fill, ink, hour], i) => {
  const at = new Date(TL_NOW); at.setHours(Math.floor(hour), (hour % 1) * 60, 0, 0);
  return {
    frameId: 5000 + i, filePath: OV_SHOT(fill, ink), capturedAtMs: at.getTime(),
    activityId: 60 + i, title, focus: null,
    activityStartedAtMs: at.getTime() - 1800000, activityEndedAtMs: at.getTime() + 1800000,
    durationMs: 3600000,
  };
});
const OV_CONVERSATIONS = [
  ["Launch sync", 13.03, 2280000, 5], ["Design review", 10.2, 2460000, 3],
  ["1:1 with Priya", 9.5, 1320000, 2],
].map(([title, hour, spokenMs, speakerCount], i) => {
  const at = new Date(TL_NOW); at.setHours(Math.floor(hour), Math.round((hour % 1) * 100), 0, 0);
  return {
    activityId: 70 + i, title, startedAtMs: at.getTime(),
    endedAtMs: at.getTime() + spokenMs, spokenMs, speakerCount,
  };
});
const OV_ASKS = [
  ["What did Priya say about the webhook?", 4, 13.58],
  ["Where did the afternoon go?", 2, 15.2],
].map(([title, turnCount, hour], i) => {
  const at = new Date(TL_NOW); at.setHours(Math.floor(hour), Math.round((hour % 1) * 100), 0, 0);
  return {
    conversationId: "conv-" + i, title, origin: "quick_recall",
    createdAtMs: at.getTime(), updatedAtMs: at.getTime(), turnCount, preview: title,
  };
});
const OV_DIGEST = {
  rangeKind: "day", rangeStartMs: TL_NOW - 43200000, rangeEndMs: TL_NOW,
  headline: "A licensing day that ended in the webhook log.",
  narrative: "Mostly licensing: the Polar webhook fix landed after the launch sync, and the " +
    "afternoon went to answer_view.rs and the round-4 mockups. One open thread — Tom wants " +
    "delivery ids in the webhook log line before the dashboard can tell a flake from a real failure.",
  generatedAtMs: TL_NOW - 1500000,
};
const OV_FACTS = {
  capturePath: "/Users/you/Movies/Mnema", diskFreeBytes: 214000000000,
  totalRamBytes: 17179869184, measuredBytesPerDay: 1350000000, measuredDays: 7,
  screenFrameRate: 0.5, ocrBacklog: 0, transcriptionBacklog: 0,
  semanticVectorCount: null, semanticPendingCount: null, semanticVectorBytes: 768,
  databaseBytes: 2400000000,
};
const OV_USAGE = {
  rangeStartMs: TL_NOW - 43200000, rangeEndMs: TL_NOW, timePerSite: [],
  appTransitions: [], activityHeatmap: [],
  timePerApp: TL_APPS.map(([appBundleId, app], i) => ({
    app, appBundleId, activeMs: (6 - i) * 1800000, frameCount: 12000 - i * 1900,
  })),
};
const OV_CONTEXT = {
  engineAvailable: true, activityCount: 386, conclusionCount: 142,
  lastDerivedAtMs: TL_NOW - 900000, coveredUntilMs: TL_NOW, backfilling: false,
  tokenUsage: { inputTokens: 0, outputTokens: 0, totalTokens: 0 },
  budgetTier: "balanced", subjectCount: 12, dismissedCount: 3, skippedWindows24h: 2,
};

// ── Recording-settings fixture (additive) ────────────────────────────────────
// Without this, get_recording_settings returned {} and every settings panel
// sat on "Loading settings…" — worse, a bindable prop then took undefined and
// Svelte threw props_invalid_value on EVERY re-render, which silently aborted
// the render and made the surface look frozen (⌘F filtering appeared broken
// when it was only unable to commit). Shipping-default-shaped, so a screenshot
// shows the app as a new user meets it.
const TL_MIC = { id: "BuiltInMicrophoneDevice", name: "MacBook Pro Microphone",
  isDefault: true, isAvailable: true, transport: "builtin" };
const TL_SETTINGS = {
  captureScreen: true, captureMicrophone: true, captureSystemAudio: true,
  segmentDurationSeconds: 300, screenFrameRate: 0.5,
  saveDirectory: "/Users/you/.mnema", autoStart: false,
  screenResolution: { mode: "preset", preset: "original" },
  videoBitrate: { mode: "preset", preset: "medium", customMbps: null },
  nativeCaptureDebugLoggingEnabled: false,
  pauseCaptureOnInactivity: true, idleTimeoutSeconds: 180,
  activityMode: "input", microphoneActivitySensitivity: 0.5,
  systemAudioActivitySensitivity: 0.5, microphoneVadAdapter: "webrtc",
  audioSpeechDetection: { detector: "webrtc" },
  metadata: { enabled: true, browserUrlMode: "off" },
  privacy: { excludedApps: [], filterSystemAudio: true },
  access: { askAiEnabled: true, askAiMaxToolCalls: 12, askAiWebFetchEnabled: false },
  aiRuntime: { enabled: false, providers: [], defaultModel: null, mcpServers: [] },
  userContext: { enabled: false, derivationBudgetTier: "balanced", backfillWindowDays: 7, backfillGoDeeper: false },
  semanticSearch: { enabled: SEM === "on", provider: "candle", modelId: SEM === "on" ? "nomic-embed-text-v1.5" : null },
  previewCacheTtlSeconds: 60, followTimelineLive: true,
  retentionPolicy: "never", appearance: "system",
  ocr: { enabled: true, provider: "apple_vision", modelId: null, language: null,
    recognitionMode: "accurate", languageCorrection: true,
    tesseractPageSegmentationMode: "auto", tesseractPreprocessMode: "grayscale",
    tesseractUpscaleFactor: 1, tesseractCharWhitelist: null },
  transcription: { enabled: true, microphoneEnabled: true, systemAudioEnabled: true,
    provider: "parakeet", modelId: null, language: "en", memoryMode: "balanced",
    idleUnloadSeconds: 120, chunkSeconds: 30 },
  speakerAnalysis: { separateSpeakers: true, recognizeSavedPeople: false,
    autoLabelOwner: false, provider: "speakrs", modelId: null, timeoutSeconds: 120 },
  developerOptionsEnabled: false,
};

// ── Transcription model fixture (additive) ───────────────────────────────────
// The model picker instrument draws one row per model of the selected provider
// with its download size priced against this Mac, so an empty provider list
// left it with nothing to render. Sizes are the real manifest figures; the
// provider set matches ADR 0047 (deepgram is a fourth provider, cloud).
const TX_MODEL = (provider, modelId, displayName, byteSize, management = "app_managed", status = "installed") => ({
  provider, modelId, displayName,
  description: displayName + " — fixture status for the render harness.",
  management, status, available: status === "installed" || status === "os_managed",
  availabilityStatus: null, installPath: byteSize ? "/Users/you/.mnema/models/" + modelId : null,
  missingFiles: [], failureMessage: null, licenseLabel: "Apache-2.0", sourceUrl: null,
  download: byteSize ? { byteSize, sha256: "0".repeat(64), url: "https://example.invalid" } : null,
});
const TX_MODELS = { modelsDirectory: "/Users/you/.mnema/models", providers: [
  { provider: "parakeet", displayName: "Parakeet", models: [
    TX_MODEL("parakeet", "parakeet-tdt-0.6b-v3-onnx-int8", "Parakeet v3 (int8)", 620000000),
    TX_MODEL("parakeet", "parakeet-tdt-0.6b-v3-onnx", "Parakeet v3", 2400000000, "app_managed", "missing"),
  ] },
  { provider: "local_whisper", displayName: "Local Whisper", models: [
    TX_MODEL("local_whisper", "base", "Whisper base", 148000000),
    TX_MODEL("local_whisper", "large-v3-turbo", "Whisper large-v3 turbo", 1600000000, "app_managed", "missing"),
    TX_MODEL("local_whisper", "large-v3", "Whisper large-v3", 6200000000, "app_managed", "missing"),
  ] },
  { provider: "apple_speech_on_device", displayName: "Apple Speech", models: [
    TX_MODEL("apple_speech_on_device", null, "Apple Speech (on-device)", 0, "os_managed", "os_managed"),
  ] },
  { provider: "deepgram", displayName: "Deepgram", models: [
    TX_MODEL("deepgram", "nova-3", "Deepgram nova-3", 0, "cloud", "missing"),
  ] },
] };

window.__TAURI_INTERNALS__ = {
  invoke: async (cmd, args) => {
    const req = (args && (args.request ?? args.payload ?? args)) || {};
    if (cmd === "take_pending_license_deep_link") return null;          // the one legal null
    if (cmd === "get_moments") return OV_EMPTY ? [] : OV_MOMENTS;
    if (cmd === "get_conversations") return OV_EMPTY ? [] : OV_CONVERSATIONS;
    if (cmd === "list_conversations") return OV_EMPTY ? [] : OV_ASKS;
    if (cmd === "get_latest_user_context_digest") return OV_EMPTY ? null : OV_DIGEST;
    if (cmd === "get_user_context_status") return OV_EMPTY ? { ...OV_CONTEXT, activityCount: 0, conclusionCount: 0, subjectCount: 0 } : OV_CONTEXT;
    if (cmd === "get_system_facts") return OV_EMPTY ? { ...OV_FACTS, diskFreeBytes: null, measuredBytesPerDay: null, measuredDays: 0 } : { ...OV_FACTS, ...SEM_FACTS };
    if (cmd === "get_usage_charts") return OV_EMPTY ? { ...OV_USAGE, timePerApp: [] } : OV_USAGE;
    if (cmd === "list_frames") return req.beforeId ? [] : TL_FRAMES;
    if (cmd === "get_frame") return TL_FRAMES[0];
    if (cmd === "get_latest_frame_in_range") return TL_FRAMES[0];
    if (cmd === "get_frame_preview" || cmd === "get_scrub_preview")
      return { mimeType: "image/svg+xml", filePath: TL_SHOT, sourceKind: "original_frame", hasSecretRedactions: false, secretRedactionCount: 0 };
    if (cmd === "get_scrub_preview_availability") return [];
    if (cmd === "list_audio_segments") return TL_SEGS;
    if (cmd === "get_audio_segment") return TL_SEGS[0];
    if (cmd === "list_day_coverage") return OV_EMPTY ? [] : TL_COVERAGE;
    if (cmd === "list_frame_summaries_in_range") return OV_EMPTY ? [] : TL_SUMMARIES(req);
    if (cmd === "get_audio_segment_media") return { mimeType: "audio/wav", dataBase64: TL_WAV };
    if (cmd === "get_audio_segment_waveform_peaks")
      return Array.from({ length: req.bucketCount ?? 96 }, (_, i) =>
        0.12 + 0.78 * Math.abs(Math.sin(i / 3.7)) * (0.45 + 0.55 * Math.abs(Math.cos(i / 11))));
    if (cmd === "list_speaker_turns") return TL_TURNS;
    if (cmd === "get_recording_settings") return TL_SETTINGS;
    if (cmd === "get_microphone_controller_state" || cmd === "update_microphone_controller")
      return { devices: [TL_MIC], preference: { mode: "default", deviceId: null },
               disconnectPolicy: "fallback_to_default", effectiveDevice: TL_MIC };
    if (cmd === "get_storage_location") return "/Users/you/.mnema";
    if (cmd === "get_app_notifications") return [];
    if (cmd === "get_license_status") return { licensed: { kind: "purchased", updatesUntilMs: Date.now() + 3.15e10 } };
    if (cmd === "get_audio_transcription_model_status") return TX_MODELS;
    if (cmd.startsWith("update_"))
      return { domain: req.domain ?? args?.domain ?? "capture_sources", settings: { ...TL_SETTINGS, ...req } };
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

// Optional interaction, so an INTERACTIVE state can be screenshotted too — a
// filtered settings list, an armed shortcut recorder, an open menu. Without
// this every shot is the at-rest state, and half this direction's states are
// not at rest.
//   CLICK='.some-button' TYPE_INTO='input[…]' TYPE_TEXT='theme' bun scripts/rd4-shot.mjs …
if (process.env.CLICK) {
  await page.click(process.env.CLICK);
  await page.waitForTimeout(400);
}
if (process.env.TYPE_INTO) {
  await page.click(process.env.TYPE_INTO);
  await page.type(process.env.TYPE_INTO, process.env.TYPE_TEXT ?? "", { delay: 40 });
  await page.waitForTimeout(600);
}
if (process.env.SCROLL_BY) {
  await page.evaluate((y) => {
    const el = document.querySelector(".settings-scroll, .ti-pane, main") ?? document.scrollingElement;
    el?.scrollBy(0, Number(y));
  }, process.env.SCROLL_BY);
  await page.waitForTimeout(400);
}

// Opt-in interaction before the shot, for states that only exist after a click
// or a keypress (a menu open, a drawer open). CLICK is a selector, PRESS a key
// name; SUFFIX names the PNG so two states of one route don't collide.
if (process.env.CLICK) {
  for (const sel of process.env.CLICK.split("|")) {
    await page.locator(sel.trim()).first().click({ timeout: 5000 }).catch((e) => errors.push(`CLICK ${sel}: ${e.message}`));
    await page.waitForTimeout(500);
  }
  await page.waitForTimeout(600);
}
if (process.env.PRESS) {
  for (const key of process.env.PRESS.split("|")) {
    await page.keyboard.press(key.trim()).catch((e) => errors.push(`PRESS ${key}: ${e.message}`));
    await page.waitForTimeout(700);
  }
  await page.waitForTimeout(600);
}

// Settings is one long scrolling column, so most sections are below the fold.
// SCROLL is a selector scrolled to the top of its pane before the shot — the
// only way to photograph a section that is not the first one, and the only way
// to prove the sticky group header is real rather than asserted.
if (process.env.SCROLL) {
  await page
    .locator(process.env.SCROLL)
    .first()
    .evaluate((el) => el.scrollIntoView({ block: "start" }))
    .catch((e) => errors.push(`SCROLL ${process.env.SCROLL}: ${e.message}`));
  await page.waitForTimeout(500);
}

const name = `${route.replace(/\W+/g, "_") || "root"}${process.env.SUFFIX ? "-" + process.env.SUFFIX : ""}-${theme}-${size}.png`;
// REPORT=1 prints a small DOM census before the browser closes, so a check can
// assert on BEHAVIOUR (did ⌘F actually filter?) rather than on a class name.
const report = process.env.REPORT === "1"
  ? await page.evaluate(() => ({
      rows: document.querySelectorAll(".setting-row").length,
      miss: document.querySelectorAll(".setting-row--miss").length,
      crumbs: document.querySelectorAll(".setting-row__crumb").length,
      finding: document.querySelector(".settings-shell")?.classList.contains("is-finding") ?? null,
    }))
  : null;

const out = path.join(outDir, name);
await page.screenshot({ path: out, fullPage: false });
await browser.close();

console.log(out);
if (errors.length) console.log("PAGE ERRORS:\n" + errors.slice(0, 8).join("\n"));
if (report) console.log("REPORT " + JSON.stringify(report));
