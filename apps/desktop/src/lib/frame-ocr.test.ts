// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { ocrBoxStyle, parseOcrPayload } from "./frame-ocr";

describe("parseOcrPayload", () => {
  test("valid payload → observations + provider label", () => {
    const json = JSON.stringify({
      provider: "apple_vision",
      modelId: "v3",
      observations: [
        { text: "hello", confidence: 0.9, boundingBox: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 } },
      ],
    });
    const out = parseOcrPayload(json);
    expect(out).not.toBeNull();
    expect(out!.observations).toEqual([
      { text: "hello", confidence: 0.9, boundingBox: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 } },
    ]);
    expect(out!.providerLabel).toBe("Apple Vision · v3");
  });

  test("provider label falls back to provenance and formats known providers", () => {
    const json = JSON.stringify({
      provenance: { provider: "paddle_ocr" },
      observations: [],
    });
    const out = parseOcrPayload(json);
    expect(out!.providerLabel).toBe("PaddleOCR");
  });

  test("unknown provider passes through verbatim, no modelId → no suffix", () => {
    const json = JSON.stringify({ provider: "some_engine", observations: [] });
    expect(parseOcrPayload(json)!.providerLabel).toBe("some_engine");
  });

  test("null/empty input → null", () => {
    expect(parseOcrPayload(null)).toBeNull();
    expect(parseOcrPayload("")).toBeNull();
  });

  test("malformed JSON → null", () => {
    expect(parseOcrPayload("{not json")).toBeNull();
  });

  test("missing observations array → null", () => {
    expect(parseOcrPayload(JSON.stringify({ provider: "apple_vision" }))).toBeNull();
  });

  test("observation with missing/non-number bbox fields is skipped", () => {
    const json = JSON.stringify({
      observations: [
        { text: "keep", confidence: 1, boundingBox: { x: 0, y: 0, width: 1, height: 1 } },
        { text: "no bbox", confidence: 1 },
        { text: "bad bbox", confidence: 1, boundingBox: { x: "0", y: 0, width: 1, height: 1 } },
      ],
    });
    const out = parseOcrPayload(json)!;
    expect(out.observations.map((o) => o.text)).toEqual(["keep"]);
    expect(out.providerLabel).toBeNull();
  });

  test("non-string text and non-number confidence default to '' and 0", () => {
    const json = JSON.stringify({
      observations: [{ boundingBox: { x: 0, y: 0, width: 1, height: 1 } }],
    });
    const out = parseOcrPayload(json)!;
    expect(out.observations[0]).toEqual({
      text: "",
      confidence: 0,
      boundingBox: { x: 0, y: 0, width: 1, height: 1 },
    });
  });
});

describe("ocrBoxStyle", () => {
  test("percent geometry, lower-left→top y-flip, font-size from box height", () => {
    const obs = {
      text: "x",
      confidence: 1,
      boundingBox: { x: 0.25, y: 0.1, width: 0.5, height: 0.2 },
    };
    // top = (1 - 0.1 - 0.2) * 100 = 70
    // heightPx = max(8, 0.2 * 1000) = 200 ; fontSize = max(6, 200 * 0.78) = 156
    expect(ocrBoxStyle(obs, 1000)).toBe(
      "left: 25%; top: 70%; width: 50%; height: 20%; --ocr-font-size: 156.00px;",
    );
  });

  test("font-size floors: tiny box clamps heightPx≥8 then fontSize≥6", () => {
    const obs = {
      text: "x",
      confidence: 1,
      boundingBox: { x: 0, y: 0, width: 1, height: 0.001 },
    };
    // heightPx = max(8, 0.001 * 100) = 8 ; fontSize = max(6, 8 * 0.78 = 6.24) = 6.24
    expect(ocrBoxStyle(obs, 100)).toBe(
      "left: 0%; top: 99.9%; width: 100%; height: 0.1%; --ocr-font-size: 6.24px;",
    );
  });
});
