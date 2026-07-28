// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
//
// The mockup's `console.assert` self-check, ported: every figure this component
// prints is recomputed from the manifest byte counts, so the numbers fail loudly
// if the manifests move.
import { describe, expect, it } from "bun:test";
import {
  RECOMMENDED_ENGINE,
  buildEngines,
  downloadLabel,
  engineDelta,
  memoryLabel,
  trackWidth,
} from "./transcription-engines";

// Manifest figures — `crates/audio-transcription/src/lib.rs`.
const WHISPER_TINY = 77_691_713;
const WHISPER_BASE = 147_951_465;
const PARAKEET_INT8 = 670_619_803;
const PARAKEET_FULL = 2_549_945_719;

const model = (modelId, byteSize, available = false) => ({
  modelId,
  available,
  download: byteSize === null ? null : { byteSize },
});

const status = (overrides = {}) => [
  {
    provider: "local_whisper",
    displayName: "Local Whisper",
    models: [model("tiny", WHISPER_TINY), model("base", WHISPER_BASE), ...(overrides.whisper ?? [])],
    ...(overrides.whisperProvider ?? {}),
  },
  {
    provider: "apple_speech_on_device",
    displayName: "Apple Speech (on-device)",
    models: [model(null, null, true)],
  },
  {
    provider: "parakeet",
    displayName: "Parakeet",
    models: [
      model("parakeet-tdt-0.6b-v3-onnx", PARAKEET_FULL),
      model("parakeet-tdt-0.6b-v3-onnx-int8", PARAKEET_INT8),
    ],
  },
  {
    provider: "deepgram",
    displayName: "Deepgram (cloud)",
    models: [model("nova-3", null)],
  },
];

const byId = (engines, id) => engines.find((e) => e.id === id);

describe("buildEngines", () => {
  it("never offers Deepgram — cloud transcription is Settings-only (ADR 0047)", () => {
    expect(buildEngines(status()).map((e) => e.id)).toEqual([
      "local_whisper",
      "apple_speech_on_device",
      "parakeet",
    ]);
  });

  it("takes each engine's cost from its DEFAULT build, not its biggest", () => {
    const engines = buildEngines(status());
    expect(byId(engines, "local_whisper").bytes).toBe(WHISPER_BASE);
    // The int8 build is the default; "Parakeet is huge" is only true of the one
    // this screen does not select.
    expect(byId(engines, "parakeet").bytes).toBe(PARAKEET_INT8);
  });

  it("prints the full-precision Parakeet build and its extra, both computed", () => {
    expect(byId(buildEngines(status()), "parakeet").foot).toBe(
      "full-precision build 2.5 GB (+1.9 GB) — chosen in the model picker, not here",
    );
  });

  it("gives the recommended engine no footnote about a bigger build", () => {
    expect(byId(buildEngines(status()), "local_whisper").foot).toBeNull();
  });

  it("reads 'no download' for the OS-managed engine", () => {
    expect(downloadLabel(byId(buildEngines(status()), "apple_speech_on_device"))).toBe(
      "no download",
    );
  });

  it("reads the real size for an engine that still has to fetch its model", () => {
    expect(downloadLabel(byId(buildEngines(status()), "local_whisper"))).toBe("148.0 MB");
  });

  it("charges nothing for a model already on this Mac", () => {
    const installed = status();
    installed[0].models = [model("tiny", WHISPER_TINY), model("base", WHISPER_BASE, true)];
    const whisper = byId(buildEngines(installed), "local_whisper");
    expect(whisper.bytes).toBe(0);
    expect(downloadLabel(whisper)).toBe("already on this Mac");
  });

  it("names the engines but claims no size before the status arrives", () => {
    const engines = buildEngines([]);
    expect(engines.map((e) => e.name)).toEqual(["Apple Speech", "Whisper", "Parakeet"]);
    expect(engines.every((e) => e.bytes === null)).toBe(true);
    expect(downloadLabel(engines[0])).toBe("checking…");
  });

  it("invents no memory figure for an engine nobody measured", () => {
    const engines = buildEngines([
      { provider: "future_engine", displayName: "Future", models: [model("m", 1_000)] },
    ]);
    expect(engines[0].ramBytes).toBeNull();
    expect(memoryLabel(engines[0])).toBe("not measured");
    expect(trackWidth(engines[0].ramBytes, 3_000_000_000)).toBe(0);
  });
});

describe("engineDelta", () => {
  it("asks the user to decide nothing on the recommended engine", () => {
    expect(engineDelta(buildEngines(status()), RECOMMENDED_ENGINE)).toEqual({
      text: "This is what Mnema picked. Nothing to decide unless you want to.",
      up: false,
    });
  });

  it("prices Parakeet against the recommendation on both measured axes", () => {
    expect(engineDelta(buildEngines(status()), "parakeet")).toEqual({
      text: "+522.7 MB to download · about +1.6 GB of memory vs the recommended engine",
      up: true,
    });
  });

  it("shows Apple Speech as the cheaper choice it is", () => {
    expect(engineDelta(buildEngines(status()), "apple_speech_on_device")).toEqual({
      text: "−148.0 MB to download · about −600.0 MB of memory vs the recommended engine",
      up: false,
    });
  });

  it("leaves an unknown axis out of the sentence instead of guessing it", () => {
    const delta = engineDelta(buildEngines([]), "parakeet");
    // Nothing downloaded is known yet, so only the measured memory delta speaks.
    expect(delta.text).toBe("about +1.6 GB of memory vs the recommended engine");
  });
});
