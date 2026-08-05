// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { FramePreviewUrlHolder, FramePreviewUrlMap } from "./frame-preview";

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

/** A url map wired to fakes, plus the fetch/revoke logs the assertions read. */
function mapHarness(capacity: number) {
  const revoked: string[] = [];
  const fetched: string[] = [];
  let minted = 0;
  const urls = new FramePreviewUrlMap(
    {
      convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
      fetchImpl: async (url: string) => {
        fetched.push(url);
        return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
      },
      createObjectUrlImpl: () => `blob:${++minted}`,
      revokeObjectUrlImpl: (url: string) => {
        revoked.push(url);
      },
    },
    capacity,
  );
  return { urls, revoked, fetched };
}

/** `n` thumbnail entries, frame ids 1..n. */
function entries(from: number, to: number) {
  const out = [];
  for (let id = from; id <= to; id++) {
    out.push({ frameId: id, preview: { filePath: `/frames/${id}.png`, mimeType: "image/png" } });
  }
  return out;
}

describe("FramePreviewUrlMap", () => {
  test("the live set is capped, and eviction revokes what it drops", async () => {
    // The whole point: an afternoon of scrolling must not grow the decoded-surface
    // set without bound. Past the cap the oldest URL is revoked, not just dropped.
    const { urls, revoked } = mapHarness(2);

    await urls.merge(entries(1, 2));
    expect(revoked).toEqual([]);
    expect([...urls.snapshot().keys()]).toEqual([1, 2]);

    await urls.merge(entries(3, 3));
    expect(revoked).toEqual(["blob:1"]);
    expect([...urls.snapshot().keys()]).toEqual([2, 3]);
  });

  test("a frame already held is neither re-fetched nor re-minted", async () => {
    const { urls, fetched } = mapHarness(8);

    const first = await urls.merge(entries(1, 2));
    await urls.merge(entries(2, 3));

    expect(fetched).toEqual(["asset:///frames/1.png", "asset:///frames/2.png", "asset:///frames/3.png"]);
    expect(urls.snapshot().get(2)).toBe(first.get(2));
  });

  test("clear revokes every live URL", async () => {
    const { urls, revoked } = mapHarness(8);

    await urls.merge(entries(1, 3));
    urls.clear();

    expect(revoked).toEqual(["blob:1", "blob:2", "blob:3"]);
    expect(urls.snapshot().size).toBe(0);
  });

  test("one frame failing to fetch leaves the rest painted", async () => {
    // Thumbnails are best-effort: a broken frame must not blank its neighbours,
    // and must stay out of the map so a later pass can retry it.
    const revoked: string[] = [];
    let minted = 0;
    const urls = new FramePreviewUrlMap({
      convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
      fetchImpl: async (url: string) =>
        url.includes("/2.png")
          ? { ok: false, status: 404, statusText: "Not Found" }
          : { ok: true, arrayBuffer: async () => new ArrayBuffer(4) },
      createObjectUrlImpl: () => `blob:${++minted}`,
      revokeObjectUrlImpl: (url: string) => revoked.push(url),
    });

    const painted = await urls.merge(entries(1, 3));

    expect([...painted.keys()]).toEqual([1, 3]);
    expect(revoked).toEqual([]);
  });
});
