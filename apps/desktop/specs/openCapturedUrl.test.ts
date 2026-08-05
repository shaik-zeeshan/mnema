import { beforeEach, describe, expect, mock, test } from "bun:test";

// Mock the surfaces the helper depends on BEFORE importing it, so the SUT binds
// these mocks at import time. `invoke` drives the brokered open outcome; `toast`
// is the feedback sink (the app-wide toast placement — never window.alert, and
// no longer a modal dialog). The rune store itself can't be imported under bun.
// The reviewers verified `mock.module` works under this repo's bun.
const invoke = mock(
  async (_cmd: string, _args?: unknown): Promise<unknown> => false,
);
const message = mock(async (_msg: string, _opts?: unknown): Promise<void> => {});
const toast = mock((_input: unknown): string => "id");

mock.module("@tauri-apps/api/core", () => ({
  invoke,
  // bun module mocks fix the export-name set process-wide; later test files
  // transitively import convertFileSrc (frame-preview), so it must exist here.
  convertFileSrc: (p: string) => p,
}));
// Same export-name-set rule as convertFileSrc above: controller-mcp.test.ts
// (later in the run) imports `confirm`, so it must exist in this first-
// registered dialog mock; that file re-mocks the implementations it asserts on.
mock.module("@tauri-apps/plugin-dialog", () => ({
  message,
  confirm: async () => true,
  ask: async () => false,
}));

mock.module("$lib/toast.svelte", () => ({ toast }));

const { openCapturedUrl } = await import("../src/lib/open-captured-url");

beforeEach(() => {
  invoke.mockReset();
  message.mockReset();
  toast.mockReset();
});

describe("openCapturedUrl 3-state contract", () => {
  test("opened: invoke -> true returns {status:'opened'} and pops NO dialog", async () => {
    invoke.mockImplementation(async () => true);

    const result = await openCapturedUrl(42);

    expect(result).toEqual({ status: "opened" });
    // The producer/consumer contract with the Rust `open_captured_url` command:
    // command name + the `{ frameId }` arg shape.
    expect(invoke).toHaveBeenCalledWith("open_captured_url", { frameId: 42 });
    expect(toast).not.toHaveBeenCalled();
  });

  test("no-url: invoke -> false returns {status:'no-url'} and raises the info toast", async () => {
    invoke.mockImplementation(async () => false);

    const result = await openCapturedUrl(7);

    expect(result).toEqual({ status: "no-url" });
    expect(toast).toHaveBeenCalledWith({
      id: "open-captured-url",
      title: "Couldn't open page",
      message: "No openable page for this result.",
    });
  });

  test("error (string): invoke throws a string -> {status:'error',error:<string>} + error toast", async () => {
    invoke.mockImplementation(async () => {
      throw "broker exploded";
    });

    const result = await openCapturedUrl(7);

    expect(result).toEqual({ status: "error", error: "broker exploded" });
    expect(toast).toHaveBeenCalledWith({
      id: "open-captured-url",
      tone: "error",
      title: "Couldn't open page",
      message: "Couldn't open URL: broker exploded",
    });
  });

  test("error (non-string): invoke throws an Error -> falls back to the generic copy", async () => {
    invoke.mockImplementation(async () => {
      throw new Error("opaque internal failure");
    });

    const result = await openCapturedUrl(7);

    // The negative-space fallback string — nothing else exercises it.
    expect(result).toEqual({
      status: "error",
      error: "the page could not be opened",
    });
    expect(toast).toHaveBeenCalledWith({
      id: "open-captured-url",
      tone: "error",
      title: "Couldn't open page",
      message: "Couldn't open URL: the page could not be opened",
    });
  });
});

describe("openCapturedUrl silent mode (dashboard contract)", () => {
  test("silent suppresses the no-url toast but still returns the status", async () => {
    invoke.mockImplementation(async () => false);

    const result = await openCapturedUrl(7, { silent: true });

    expect(result).toEqual({ status: "no-url" });
    expect(toast).not.toHaveBeenCalled();
  });

  test("silent suppresses the error toast but still returns status + error", async () => {
    invoke.mockImplementation(async () => {
      throw "kaboom";
    });

    const result = await openCapturedUrl(7, { silent: true });

    expect(result).toEqual({ status: "error", error: "kaboom" });
    expect(toast).not.toHaveBeenCalled();
  });
});
