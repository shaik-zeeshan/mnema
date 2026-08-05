// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { modelFit } from "./model-fit";

const GB = 1_000_000_000;
const RAM_16 = 17_179_869_184; // what macOS reports for a "16 GB" Mac

describe("modelFit", () => {
  it("has nothing to weigh for a cloud or OS-managed model", () => {
    expect(modelFit(null, RAM_16, 200 * GB)).toBeNull();
    expect(modelFit(0, RAM_16, 200 * GB)).toBeNull();
  });

  it("renders a bar but no verdict when RAM is unreadable (G8)", () => {
    expect(modelFit(620_000_000, null, 200 * GB)).toEqual({
      ramPercent: null,
      verdict: null,
      tone: null,
    });
  });

  it("grades against this Mac's RAM", () => {
    expect(modelFit(620_000_000, RAM_16, 200 * GB).tone).toBe("ok");
    expect(modelFit(3 * GB, RAM_16, 200 * GB).tone).toBe("warn");
    expect(modelFit(6.2 * GB, RAM_16, 200 * GB).tone).toBe("danger");
  });

  it("reports the model's share of RAM", () => {
    const fit = modelFit(RAM_16 / 2, RAM_16, 200 * GB);
    expect(Math.round(fit.ramPercent)).toBe(50);
  });

  it("checks disk before RAM — a model that cannot land is not a RAM question", () => {
    const fit = modelFit(3 * GB, RAM_16, 1 * GB);
    expect(fit.verdict).toBe("not enough free disk");
    expect(fit.tone).toBe("danger");
  });
});
