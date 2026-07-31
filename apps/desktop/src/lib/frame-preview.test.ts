// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { FramePreviewUrlHolder } from "./frame-preview";

/** A holder wired to fakes, plus the revoke log the assertions read. */
function harness(options: { fetchImpl?: typeof fetch } = {}) {
  const revoked: string[] = [];
  let minted = 0;
  const holder = new FramePreviewUrlHolder({
    convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
    fetchImpl:
      options.fetchImpl ??
      (async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) })),
    createObjectUrlImpl: () => `blob:${++minted}`,
    revokeObjectUrlImpl: (url: string) => {
      revoked.push(url);
    },
  });
  return { holder, revoked };
}

describe("FramePreviewUrlHolder", () => {
  test("only one URL is live once each swap paints", async () => {
    const { holder, revoked } = harness();

    expect(await holder.swap("/frames/1.png")).toBe("blob:1");
    holder.settle();
    expect(revoked).toEqual([]); // nothing replaced yet

    expect(await holder.swap("/frames/2.png")).toBe("blob:2");
    // The old URL survives until the new one paints — revoking it earlier
    // blanks the stage mid-scrub.
    expect(revoked).toEqual([]);
    holder.settle();
    expect(revoked).toEqual(["blob:1"]);
    expect(holder.current).toBe("blob:2");
  });

  test("swaps that outrun paints still retire every replaced URL", async () => {
    const { holder, revoked } = harness();

    await holder.swap("/frames/1.png");
    await holder.swap("/frames/2.png");
    await holder.swap("/frames/3.png");
    holder.settle();

    expect(revoked).toEqual(["blob:1", "blob:2"]);
    expect(holder.current).toBe("blob:3");
  });

  test("a swap superseded mid-fetch mints nothing", async () => {
    let release: (() => void) | null = null;
    const slow = new Promise<void>((resolve) => {
      release = resolve;
    });
    let call = 0;
    const { holder, revoked } = harness({
      fetchImpl: async () => {
        if (++call === 1) await slow;
        return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
      },
    });

    const stale = holder.swap("/frames/1.png");
    expect(await holder.swap("/frames/2.png")).toBe("blob:1");
    release!();

    expect(await stale).toBeNull();
    // The superseded swap never created a URL, so the painted one stands and
    // there is nothing stranded to revoke.
    expect(holder.current).toBe("blob:1");
    holder.settle();
    expect(revoked).toEqual([]);
  });

  test("clear revokes the painted URL and disarms in-flight swaps", async () => {
    let release: (() => void) | null = null;
    const slow = new Promise<void>((resolve) => {
      release = resolve;
    });
    let call = 0;
    const { holder, revoked } = harness({
      fetchImpl: async () => {
        if (++call === 2) await slow;
        return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
      },
    });

    await holder.swap("/frames/1.png");
    const inFlight = holder.swap("/frames/2.png");
    holder.clear();
    release!();

    expect(revoked).toEqual(["blob:1"]);
    expect(await inFlight).toBeNull();
    expect(holder.current).toBeNull();
  });

  test("a failed fetch surfaces to the caller and leaves the painted URL alone", async () => {
    let call = 0;
    const { holder, revoked } = harness({
      fetchImpl: async () => {
        if (++call === 2) return { ok: false, status: 404, statusText: "Not Found" };
        return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
      },
    });

    await holder.swap("/frames/1.png");
    expect(holder.swap("/frames/2.png")).rejects.toThrow("frame preview fetch failed");
    expect(holder.current).toBe("blob:1");
    expect(revoked).toEqual([]);
  });
});
