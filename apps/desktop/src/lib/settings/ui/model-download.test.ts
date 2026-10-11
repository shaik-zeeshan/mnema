// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, test } from "bun:test";
import { downloadMeta } from "./model-download";

describe("downloadMeta", () => {
  test("known total → bytes, percent and a determinate bar", () => {
    expect(downloadMeta("Tesseract", { status: "downloading", downloadedBytes: 22e6, totalBytes: 57e6 })).toEqual({
      title: "Downloading Tesseract",
      detail: "22.0 MB of 57.0 MB · 39%",
      percent: 39,
    });
  });

  test("unknown total → indeterminate", () => {
    expect(downloadMeta("Parakeet", { status: "downloading", downloadedBytes: 5e6, totalBytes: null }).percent).toBeNull();
    expect(downloadMeta("Parakeet", { status: "downloading", downloadedBytes: 0, totalBytes: 0 }).detail).toBe("");
  });

  test("status picks the title; percent is capped at 100", () => {
    expect(downloadMeta("X", null).title).toBe("Starting download");
    expect(downloadMeta("X", { status: "starting", downloadedBytes: 0, totalBytes: 10 }).title).toBe("Starting download");
    const installing = downloadMeta("X", { status: "installing", downloadedBytes: 20, totalBytes: 10 });
    expect(installing.title).toBe("Installing X");
    expect(installing.percent).toBe(100);
  });
});
