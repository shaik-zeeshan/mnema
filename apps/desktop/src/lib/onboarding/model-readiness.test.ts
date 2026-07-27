// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  applyProgressEvent,
  classifyReadiness,
  isSetupNoteworthy,
  startProgress,
} from "./model-readiness";
import { resolveSetup } from "./resolve-setup";

// ── Test 3: the four-state classifier ──────────────────────────────────────

describe("classifyReadiness", () => {
  it("is Ready when the model is installed, whatever the stream says", () => {
    expect(classifyReadiness(true, null)).toBe("ready");
    expect(classifyReadiness(true, { status: "downloading" })).toBe("ready");
  });

  it("is Downloading for every in-flight status", () => {
    for (const status of ["starting", "downloading", "installing"]) {
      expect(classifyReadiness(false, { status })).toBe("downloading");
    }
  });

  it("is Ready once the stream reports completed, before the status reload", () => {
    expect(classifyReadiness(false, { status: "completed" })).toBe("ready");
  });

  it("is Failed on a failed download", () => {
    expect(classifyReadiness(false, { status: "failed" })).toBe("failed");
  });

  it("is Missing with no download, and after a cancel", () => {
    expect(classifyReadiness(false, null)).toBe("missing");
    expect(classifyReadiness(false, undefined)).toBe("missing");
    expect(classifyReadiness(false, { status: "cancelled" })).toBe("missing");
  });

  it("distinguishes Downloading from Missing — the bug this issue exists for", () => {
    expect(classifyReadiness(false, { status: "downloading" })).not.toBe("missing");
  });
});

describe("isSetupNoteworthy", () => {
  it("flags only Missing and Failed", () => {
    expect(isSetupNoteworthy("missing")).toBe(true);
    expect(isSetupNoteworthy("failed")).toBe(true);
    expect(isSetupNoteworthy("ready")).toBe(false);
  });

  it("Downloading does not block finishing", () => {
    expect(isSetupNoteworthy("downloading")).toBe(false);
    // ...and neither does anything else: no state is a finish gate. Continue is
    // live on arrival and never disables.
    const noteworthy = ["ready", "downloading", "missing", "failed"].filter(
      isSetupNoteworthy,
    );
    expect(noteworthy).toEqual(["missing", "failed"]);
  });
});

// ── Test 4: the aggregate progress reducer ─────────────────────────────────

const WORK = resolveSetup(
  { screen: true, microphone: true, systemAudio: true },
  { speakerAnalysis: false, whisperBase: false, semanticSearch: false },
  null,
).workList;

const [SPEAKRS, WHISPER, NOMIC] = WORK;

const event = (item, status, downloadedBytes, message = null) => ({
  provider: item.provider,
  modelId: item.modelId,
  status,
  downloadedBytes,
  totalBytes: item.bytes,
  message,
});

describe("startProgress", () => {
  it("starts at zero with every item Missing and the first one current", () => {
    const state = startProgress(WORK);
    expect(state.percent).toBe(0);
    expect(state.done).toBe(false);
    expect(state.currentLabel).toBe(SPEAKRS.label);
    expect(Object.values(state.states)).toEqual(["missing", "missing", "missing"]);
  });

  it("an empty work-list is done immediately at 100%", () => {
    const state = startProgress([]);
    expect(state.percent).toBe(100);
    expect(state.done).toBe(true);
    expect(state.currentLabel).toBeNull();
  });
});

describe("applyProgressEvent — byte weighting", () => {
  it("weights by bytes, not by item count", () => {
    // Whisper is the smallest item; finishing it must move the bar least.
    const whisperDone = applyProgressEvent(
      startProgress(WORK),
      event(WHISPER, "completed", WHISPER.bytes),
    );
    const nomicDone = applyProgressEvent(
      startProgress(WORK),
      event(NOMIC, "completed", NOMIC.bytes),
    );
    expect(whisperDone.percent).toBeLessThan(nomicDone.percent);
    expect(whisperDone.percent).toBeGreaterThan(0);
  });

  it("reaches 100% and done only when every item is ready", () => {
    let state = startProgress(WORK);
    for (const item of WORK) {
      expect(state.done).toBe(false);
      state = applyProgressEvent(state, event(item, "completed", item.bytes));
    }
    expect(state.percent).toBe(100);
    expect(state.done).toBe(true);
    expect(state.currentLabel).toBeNull();
  });

  it("ignores events for models that are not on the work-list", () => {
    const state = startProgress(WORK);
    const next = applyProgressEvent(state, {
      provider: "local_whisper",
      modelId: "medium",
      status: "downloading",
      downloadedBytes: 1_000_000_000,
      totalBytes: 1_533_763_059,
      message: null,
    });
    expect(next).toBe(state);
  });
});

describe("applyProgressEvent — concurrent streams", () => {
  it("folds interleaved events from different subsystems into one bar", () => {
    let state = startProgress(WORK);
    state = applyProgressEvent(state, event(SPEAKRS, "downloading", SPEAKRS.bytes / 2));
    state = applyProgressEvent(state, event(NOMIC, "downloading", NOMIC.bytes / 4));
    state = applyProgressEvent(state, event(WHISPER, "downloading", WHISPER.bytes));
    state = applyProgressEvent(state, event(SPEAKRS, "completed", SPEAKRS.bytes));

    expect(state.states[SPEAKRS.id]).toBe("ready");
    expect(state.states[NOMIC.id]).toBe("downloading");
    expect(state.states[WHISPER.id]).toBe("downloading");
    expect(state.received[NOMIC.id]).toBe(NOMIC.bytes / 4);
    // The label follows the first item still in flight.
    expect(state.currentLabel).toBe(WHISPER.label);
    expect(state.percent).toBeGreaterThan(0);
    expect(state.percent).toBeLessThan(100);
  });
});

describe("applyProgressEvent — out-of-order events", () => {
  it("drops a stale lower byte count", () => {
    let state = startProgress(WORK);
    state = applyProgressEvent(state, event(SPEAKRS, "downloading", 800_000));
    const high = state.percent;
    state = applyProgressEvent(state, event(SPEAKRS, "downloading", 400_000));
    expect(state.received[SPEAKRS.id]).toBe(800_000);
    expect(state.percent).toBe(high);
  });

  it("a late in-flight event cannot un-finish a completed item", () => {
    let state = startProgress(WORK);
    state = applyProgressEvent(state, event(SPEAKRS, "completed", SPEAKRS.bytes));
    state = applyProgressEvent(state, event(SPEAKRS, "downloading", 10));
    expect(state.states[SPEAKRS.id]).toBe("ready");
    expect(state.received[SPEAKRS.id]).toBe(SPEAKRS.bytes);
  });

  it("clamps a byte count that overshoots the known total", () => {
    const state = applyProgressEvent(
      startProgress(WORK),
      event(WHISPER, "downloading", WHISPER.bytes * 3),
    );
    expect(state.received[WHISPER.id]).toBe(WHISPER.bytes);
    expect(state.percent).toBeLessThan(100);
  });

  it("progress never moves backwards across a shuffled stream", () => {
    const stream = [];
    for (const item of WORK) {
      for (const fraction of [0.25, 0.5, 0.75]) {
        stream.push(event(item, "downloading", Math.floor(item.bytes * fraction)));
      }
      stream.push(event(item, "completed", item.bytes));
    }
    // Deterministic shuffle — reversal interleaves every stream out of order.
    let state = startProgress(WORK);
    let previous = 0;
    for (const next of stream.reverse()) {
      state = applyProgressEvent(state, next);
      expect(state.percent).toBeGreaterThanOrEqual(previous);
      previous = state.percent;
    }
    expect(state.percent).toBe(100);
    expect(state.done).toBe(true);
  });
});

describe("applyProgressEvent — failure", () => {
  it("surfaces the real error AT the failed item and keeps the rest going", () => {
    let state = startProgress(WORK);
    state = applyProgressEvent(state, event(SPEAKRS, "downloading", SPEAKRS.bytes / 2));
    const held = state.percent;
    state = applyProgressEvent(
      state,
      event(SPEAKRS, "failed", SPEAKRS.bytes / 2, "network error: connection reset"),
    );

    expect(state.states[SPEAKRS.id]).toBe("failed");
    expect(state.errors[SPEAKRS.id]).toBe("network error: connection reset");
    expect(state.done).toBe(false);
    // A failure never rewinds the bar.
    expect(state.percent).toBe(held);

    state = applyProgressEvent(state, event(WHISPER, "completed", WHISPER.bytes));
    expect(state.states[WHISPER.id]).toBe("ready");
    expect(state.percent).toBeGreaterThan(held);
  });

  it("falls back to a message when the stream gives none", () => {
    const state = applyProgressEvent(
      startProgress(WORK),
      event(NOMIC, "failed", 0),
    );
    expect(state.errors[NOMIC.id]).toBe("Download failed.");
  });

  it("clears the error when the item is retried", () => {
    let state = applyProgressEvent(startProgress(WORK), event(NOMIC, "failed", 0, "boom"));
    state = applyProgressEvent(state, event(NOMIC, "starting", 0));
    expect(state.errors[NOMIC.id]).toBeUndefined();
    expect(state.states[NOMIC.id]).toBe("downloading");
  });

  it("treats a cancelled download as Missing, with no error", () => {
    const state = applyProgressEvent(
      startProgress(WORK),
      event(WHISPER, "cancelled", 1_000),
    );
    expect(state.states[WHISPER.id]).toBe("missing");
    expect(state.errors[WHISPER.id]).toBeUndefined();
  });
});
