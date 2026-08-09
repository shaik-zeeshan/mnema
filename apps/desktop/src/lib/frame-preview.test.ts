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

  test("a superseded swap that fails is silent, not an error the caller acts on", async () => {
    // Callers route a rejected `swap` to their no-hero path: DetailPane calls
    // `clear()` (revoking the URL on screen) and the timeline stage runs its
    // decode-retry loop. A stale swap that fails AFTER a newer one already
    // painted must report the same "superseded" null as a stale swap that
    // succeeded — otherwise a broken frame the user scrubbed past blanks the
    // hero that loaded fine.
    let release: (() => void) | null = null;
    const slow = new Promise<void>((resolve) => {
      release = resolve;
    });
    let call = 0;
    const { holder, revoked } = harness({
      fetchImpl: async () => {
        if (++call === 1) {
          await slow;
          return { ok: false, status: 404, statusText: "Not Found" };
        }
        return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
      },
    });

    const stale = holder.swap("/frames/broken.png");
    expect(await holder.swap("/frames/good.png")).toBe("blob:1");
    release!();

    expect(await stale).toBeNull();
    expect(holder.current).toBe("blob:1");
    expect(revoked).toEqual([]);
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

  test("clear during an in-flight merge stops it minting more URLs", async () => {
    // `merge` awaits one byte read per frame, so a whole search's worth of
    // thumbnails takes many round trips. `clear()` is the last call a
    // component-scoped map ever gets (unmount teardown) — anything the merge
    // mints after it is stranded for the life of the WebContent process, the
    // exact leak blob URLs were adopted to stop.
    const revoked: string[] = [];
    let minted = 0;
    let release: (() => void) | null = null;
    const stalled = new Promise<void>((resolve) => {
      release = resolve;
    });
    let call = 0;
    const urls = new FramePreviewUrlMap({
      convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
      fetchImpl: async () => {
        if (++call === 2) await stalled; // unmount lands here
        return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
      },
      createObjectUrlImpl: () => `blob:${++minted}`,
      revokeObjectUrlImpl: (url: string) => revoked.push(url),
    });

    const merging = urls.merge(entries(1, 5));
    await Promise.resolve(); // let frame 1 land and frame 2 stall
    urls.clear(); // component destroyed
    release!();
    await merging;

    expect(urls.snapshot().size).toBe(0);
    // Every URL that was ever minted has been handed back.
    expect(new Set(revoked).size).toBe(minted);
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

  test("a cold merge fetches with bounded concurrency, not one at a time", async () => {
    // A fresh Quick Recall search merges FRAME_FETCH_LIMIT = 24 entries and the
    // caller paints nothing until the whole merge resolves. Awaiting each
    // `asset://` round trip in turn makes that wait 24 IPC latencies deep; the
    // pre-blob code assigned the map synchronously and let WebKit fetch the 24
    // in parallel. Bound the pool instead of serialising it (and instead of an
    // unbounded `Promise.all`, which would fan 24 concurrent reads at the
    // asset-protocol handler).
    let live = 0;
    let peak = 0;
    let minted = 0;
    const urls = new FramePreviewUrlMap(
      {
        convertFileSrcImpl: (filePath: string) => `asset://${filePath}`,
        fetchImpl: async () => {
          live += 1;
          peak = Math.max(peak, live);
          // Two microtask hops: long enough for every peer already scheduled to
          // enter `fetchImpl` before the first one resolves.
          await Promise.resolve();
          await Promise.resolve();
          live -= 1;
          return { ok: true, arrayBuffer: async () => new ArrayBuffer(4) };
        },
        createObjectUrlImpl: () => `blob:${++minted}`,
        revokeObjectUrlImpl: () => {},
      },
      64,
    );

    const painted = await urls.merge(entries(1, 24));

    expect(painted.size).toBe(24);
    expect(peak).toBeGreaterThan(1);
    expect(peak).toBeLessThanOrEqual(6); // FRAME_PREVIEW_MERGE_CONCURRENCY
  });
});

describe("preview blob-URL ownership", () => {
  test("every component that mints preview blob URLs revokes them on teardown", async () => {
    // A blob URL outlives the object that minted it: it stays registered on the
    // document until `revokeObjectURL`, so a destroyed component's `LruCache`
    // cap buys nothing — the URLs it still held are stranded, bytes and decoded
    // surface both. Worse than the `asset://` path this replaced, because a
    // remount mints FRESH URLs for the same frames instead of reusing one
    // cached surface per file path. So every component-scoped owner must
    // release on teardown.
    const root = new URL("..", import.meta.url).pathname; // apps/desktop/src
    const offenders: string[] = [];
    for await (const rel of new Bun.Glob("**/*.svelte").scan(root)) {
      const source = await Bun.file(`${root}${rel}`).text();
      const owner = source.match(
        /(?:const|let)\s+(\w+)\s*=\s*new FramePreviewUrl(?:Map|Holder)\(/,
      );
      if (!owner) continue;
      const id = owner[1];
      const releasesOnTeardown =
        new RegExp(`\\$effect\\(\\(\\)\\s*=>\\s*\\(\\)\\s*=>[^;]*${id}\\.clear\\(\\)`).test(
          source,
        ) || new RegExp(`onDestroy\\([\\s\\S]{0,600}?${id}\\.clear\\(\\)`).test(source);
      if (!releasesOnTeardown) offenders.push(rel);
    }
    expect(offenders).toEqual([]);
  });
});
