// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { formatBytes } from "./format";

describe("formatBytes", () => {
  it("runs past GB, so a multi-terabyte volume reads in TB", () => {
    // 1.42 TB free used to render "1322.5 GB" — the units table stopped at GB.
    expect(formatBytes(1_420_000_000_000)).toBe("1.4 TB");
    expect(formatBytes(2_000_000_000_000)).toBe("2.0 TB");
    expect(formatBytes(3_000_000_000_000_000)).toBe("3.0 PB");
  });

  it("is decimal/SI, matching the model manifests and macOS", () => {
    expect(formatBytes(419_482_724)).toBe("419.5 MB"); // speakrs
    expect(formatBytes(147_951_465)).toBe("148.0 MB"); // Whisper base
    expect(formatBytes(1_115_434_189)).toBe("1.1 GB"); // the default set
  });

  it("keeps whole bytes below 1 KB", () => {
    expect(formatBytes(1)).toBe("1 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1000)).toBe("1.0 KB");
  });

  it("never renders NaN", () => {
    expect(formatBytes(0)).toBe("unknown size");
    expect(formatBytes(-1)).toBe("unknown size");
    expect(formatBytes(NaN)).toBe("unknown size");
    expect(formatBytes(Infinity)).toBe("unknown size");
  });
});
