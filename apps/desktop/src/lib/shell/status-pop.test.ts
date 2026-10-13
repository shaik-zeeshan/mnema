// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, test } from "bun:test";
import { aiRow, captureGroup, catchingUp, diskRow, type PopInput } from "./status-pop";

const GB = 1024 ** 3;
const src = (over = {}) => ({
  requested: true,
  state: "recording",
  reason: null,
  levelDb: -20,
  idleMs: 0,
  deviceName: null,
  ...over,
});
const health = (over = {}) => ({
  recording: true,
  startedAtMs: 0,
  screen: src(),
  microphone: src(),
  systemAudio: src(),
  suspension: null,
  systemAudioTap: null,
  disk: {
    freeBytes: 182 * GB,
    reserveFloorBytes: 2 * GB,
    pauseThresholdBytes: 5 * GB,
    resumeThresholdBytes: 5.5 * GB,
  },
  ...over,
});
const tap = (over = {}) => ({
  rebuildsToday: 1,
  lastReason: null,
  lastRebuildAtMs: 0,
  lastDetail: null,
  nextSilenceCheckInMs: null,
  watchdogDelayMs: null,
  sampleRateHz: 48000,
  ...over,
});
const lane = (processor, over = {}) => ({
  processor,
  queued: 0,
  retrying: 0,
  running: 0,
  completed: 0,
  failed: 0,
  failedLast24h: 0,
  averageCompletedSecondsLast24h: null,
  lastError: null,
  ...over,
});
const index = (over = {}) => ({
  queuedFrames: 0,
  oldestQueuedMs: null,
  semanticBuilding: false,
  semanticIndexedPercent: 100,
  ...over,
});
const input = (over: Partial<PopInput> = {}): PopInput => ({
  running: true,
  loadingStart: false,
  lowDisk: false,
  health: health(),
  sysAudioReconnecting: false,
  diskSoon: false,
  index: index(),
  pipeline: [lane("ocr"), lane("audio_transcription")],
  transcriptionProvider: "parakeet",
  deepgramAuthError: null,
  engine: null,
  ...over,
});
const values = (i: PopInput) => {
  const g = captureGroup(i);
  return g.kind === "rows" ? g.rows.map((r) => [r.value.join(""), r.dot, r.tone]) : g.kind;
};
const REC = ["Recording", "fill", "ok"];

describe("mockup states", () => {
  test("ok: three recording rows, caught up, plain free space", () => {
    const i = input();
    expect(values(i)).toEqual([REC, REC, REC]);
    expect(catchingUp(i)).toEqual({ kind: "caught-up" });
    expect(diskRow(i)).toEqual({ name: "Free space", dot: "fill", tone: null, value: [{ num: "182 GB" }] });
    expect(aiRow(i)).toBeNull();
  });

  test("catching-up: reading with eta, transcribing clips, meaning bar", () => {
    const i = input({
      index: index({ queuedFrames: 38, semanticBuilding: true, semanticIndexedPercent: 92 }),
      pipeline: [lane("ocr", { averageCompletedSecondsLast24h: 3.2 }), lane("audio_transcription", { queued: 2, running: 1 })],
    });
    expect(catchingUp(i)).toEqual({
      kind: "busy",
      meaningPercent: 92,
      rows: [
        { name: "Reading", dot: "ring", tone: null, value: [{ num: "38" }, " frames", " · ", "about ", { num: "2 min" }] },
        { name: "Transcribing", dot: "ring", tone: null, value: [{ num: "3" }, " clips"] },
      ],
    });
  });

  test("display-asleep: neutral ring, audio continues", () => {
    const screen = src({ state: "waiting", reason: "capture_display_unavailable" });
    expect(captureGroup(input({ health: health({ screen }) })).rows[0]).toEqual({
      name: "Screen",
      dot: "ring",
      tone: null,
      value: ["Waiting for the display. Mic and system audio continue."],
    });
    const micOnly = health({ screen, systemAudio: src({ requested: false, state: "off", reason: "not_requested" }) });
    expect(values(input({ health: micOnly }))[0][0]).toBe("Waiting for the display. Mic continues.");
    const sysOnly = health({ screen, microphone: src({ requested: false, state: "off" }) });
    expect(values(input({ health: sysOnly }))[0][0]).toBe("Waiting for the display. System audio continues.");
    const alone = health({ screen, microphone: src({ state: "off" }), systemAudio: src({ state: "off" }) });
    expect(values(input({ health: alone }))[0][0]).toBe("Waiting for the display.");
  });

  test("sysaudio-rebuild: reconnecting after the reason, only while the tap is down", () => {
    const h = (lastReason) =>
      health({ systemAudio: src({ state: "reconnecting" }), systemAudioTap: tap({ lastReason, sampleRateHz: null }) });
    const row = (hh) => values(input({ health: hh, sysAudioReconnecting: true }))[2];
    expect(row(h("deviceChanged"))).toEqual(["Reconnecting after the output device changed", "ring", null]);
    expect(row(h("deviceDied"))[0]).toBe("Reconnecting after the output device went away");
    expect(row(h("sampleRateChanged"))[0]).toBe("Reconnecting after the headset changed mode");
    expect(row(h("excludeListMoved"))[0]).toBe("Reconnecting after an excluded app started or quit");
    expect(row(h("silenceWatchdog"))[0]).toBe("Reconnecting after a long silence");
    expect(row(h(null))[0]).toBe("Reconnecting");
    // Founder ruling 1: a watchdog rebuild with the tap up reads as Recording.
    const up = health({ systemAudio: src({ state: "reconnecting" }), systemAudioTap: tap({ lastReason: "silenceWatchdog" }) });
    expect(values(input({ health: up, sysAudioReconnecting: false }))[2]).toEqual(REC);
  });

  test("low-disk-soon: warn row with the pause threshold and Open Storage", () => {
    const i = input({ diskSoon: true, health: health({ disk: { ...health().disk, freeBytes: 6.1 * GB } }) });
    expect(diskRow(i)).toEqual({
      name: "Free space",
      dot: "fill",
      tone: "warn",
      value: [{ num: "6.1 GB" }, " left"],
      note: ["Recording pauses below ", { num: "5 GB" }, " and resumes when you free up space."],
      action: { label: "Open Storage", settings: "storage" },
    });
  });

  test("engine-offline: Deepgram unreachable and the AI row reuses the pill tip", () => {
    const tip = "Can't reach ChatGPT right now — your sign-in is fine. Resumes on its own.";
    const i = input({
      transcriptionProvider: "deepgram",
      pipeline: [lane("ocr"), lane("audio_transcription", { queued: 3, retrying: 3 })],
      engine: { label: "offline", error: true, tip },
    });
    expect(catchingUp(i)).toEqual({
      kind: "busy",
      meaningPercent: null,
      rows: [
        {
          name: "Transcribing",
          dot: "fill",
          tone: "warn",
          value: [{ num: "3" }, " clips waiting"],
          note: ["Can’t reach Deepgram. Retries on its own."],
        },
      ],
    });
    expect(aiRow(i)).toEqual({
      name: "Ask AI",
      dot: "fill",
      tone: "warn",
      value: [tip],
      action: { label: "AI settings", settings: "intelligence" },
    });
  });

  test("not-recording: stopped capture, reading keeps going, free space only", () => {
    const i = input({
      running: false,
      health: health({ recording: false, disk: { ...health().disk, pauseThresholdBytes: null, resumeThresholdBytes: null } }),
      index: index({ queuedFrames: 12 }),
      pipeline: [lane("ocr", { averageCompletedSecondsLast24h: 2 })],
    });
    expect(captureGroup(i)).toEqual({ kind: "stopped" });
    expect(catchingUp(i).rows[0].value).toEqual([{ num: "12" }, " frames", " · ", "under a minute"]);
    expect(diskRow(i).value).toEqual([{ num: "182 GB" }]);
  });
});

describe("undrawn capture rows", () => {
  const row = (over, i = {}) => values(input({ health: health({ screen: src(over) }), ...i }))[0];
  test("starting while the session spins up", () => {
    expect(captureGroup(input({ running: false, loadingStart: true }))).toEqual({ kind: "starting" });
  });
  test("unread health hides the rows", () => {
    expect(captureGroup(input({ health: null }))).toEqual({ kind: "rows", rows: [] });
  });
  test("paused", () => {
    expect(row({ state: "paused", reason: "user_paused" })).toEqual(["Paused by you", "ring", null]);
    expect(row({ state: "paused", reason: "inactivity" })).toEqual(["Paused while you’re away", "ring", null]);
    expect(row({ state: "paused", reason: "capture_low_disk" })).toEqual(["Paused until you free up space", "fill", "warn"]);
  });
  test("privacy and failures", () => {
    expect(row({ state: "waiting", reason: "privacy_filter_apply_failed" })).toEqual([
      "Privacy filter couldn’t be applied, retrying",
      "fill",
      "warn",
    ]);
    expect(row({ state: "failed", reason: "privacy_recovery_restart_required" })).toEqual([
      "Stopped. Restart recording to resume",
      "fill",
      "danger",
    ]);
    expect(row({ state: "failed" })).toEqual(["Not capturing right now", "fill", "warn"]);
  });
  test("off for this recording", () => {
    expect(row({ requested: false, state: "off", reason: "not_requested" })).toEqual(["Off for this recording", "ring", null]);
  });
});

describe("catching up", () => {
  const reading = (frames, avg) =>
    catchingUp(input({ index: index({ queuedFrames: frames }), pipeline: [lane("ocr", { averageCompletedSecondsLast24h: avg })] }))
      .rows[0].value;
  test("eta boundaries", () => {
    expect(reading(1, 59)).toEqual([{ num: "1" }, " frame", " · ", "under a minute"]);
    expect(reading(1, 60)).toEqual([{ num: "1" }, " frame", " · ", "about ", { num: "1 min" }]);
    expect(reading(2, 4020)).toEqual([{ num: "2" }, " frames", " · ", "about ", { num: "2h 14m" }]);
    expect(reading(38, null)).toEqual([{ num: "38" }, " frames"]);
  });
  test("both reads failed hides the group", () => {
    expect(catchingUp(input({ index: null, pipeline: null }))).toBeNull();
  });
  test("Deepgram rejected the key", () => {
    const t = catchingUp(
      input({
        transcriptionProvider: "deepgram",
        deepgramAuthError: "401",
        pipeline: [lane("audio_transcription", { queued: 3, retrying: 3 })],
      }),
    ).rows[0];
    expect(t.note).toEqual(["Deepgram rejected your key."]);
    expect(t.action).toEqual({ label: "Transcription settings", settings: "transcription" });
  });
  test("other provider retrying stays neutral", () => {
    const t = catchingUp(input({ pipeline: [lane("audio_transcription", { queued: 1, retrying: 1 })] })).rows[0];
    expect(t).toEqual({ name: "Transcribing", dot: "ring", tone: null, value: [{ num: "1" }, " clip"], note: ["Retrying on its own."] });
  });
});

describe("disk and AI", () => {
  test("paused for low disk", () => {
    const r = diskRow(input({ lowDisk: true, health: health({ disk: { ...health().disk, freeBytes: 4.2 * GB } }) }));
    expect(r.value).toEqual([{ num: "4.2 GB" }, " left"]);
    expect(r.note).toEqual(["Recording is paused. It resumes on its own once you free up space."]);
  });
  test("unreadable free space omits the row", () => {
    expect(diskRow(input({ health: null }))).toBeNull();
  });
  test("engine tones", () => {
    expect(aiRow(input({ engine: { label: "needs reconnect", error: true, tip: "x" } })).tone).toBe("danger");
    expect(aiRow(input({ engine: { label: "not set up", error: false, tip: "x" } })).tone).toBeNull();
  });
});
