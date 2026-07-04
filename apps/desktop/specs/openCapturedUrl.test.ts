import { beforeEach, describe, expect, mock, test } from "bun:test";

// Mock the Tauri surfaces the helper depends on BEFORE importing it, so the SUT
// binds these mocks at import time. `invoke` drives the brokered open outcome;
// `message` is the plugin-dialog feedback sink (never window.alert — see the
// project rule). The reviewers verified `mock.module` works under this repo's bun.
const invoke = mock(
  async (_cmd: string, _args?: unknown): Promise<unknown> => false,
);
const message = mock(async (_msg: string, _opts?: unknown): Promise<void> => {});

mock.module("@tauri-apps/api/core", () => ({
  invoke,
  // bun module mocks fix the export-name set process-wide; later test files
  // transitively import convertFileSrc (frame-preview), so it must exist here.
  convertFileSrc: (p: string) => p,
}));
mock.module("@tauri-apps/plugin-dialog", () => ({ message }));

const { openCapturedUrl } = await import("../src/lib/open-captured-url");

beforeEach(() => {
  invoke.mockReset();
  message.mockReset();
});

describe("openCapturedUrl 3-state contract", () => {
  test("opened: invoke -> true returns {status:'opened'} and pops NO dialog", async () => {
    invoke.mockImplementation(async () => true);

    const result = await openCapturedUrl(42);

    expect(result).toEqual({ status: "opened" });
    // The producer/consumer contract with the Rust `open_captured_url` command:
    // command name + the `{ frameId }` arg shape.
    expect(invoke).toHaveBeenCalledWith("open_captured_url", { frameId: 42 });
    expect(message).not.toHaveBeenCalled();
  });

  test("no-url: invoke -> false returns {status:'no-url'} and pops the info note", async () => {
    invoke.mockImplementation(async () => false);

    const result = await openCapturedUrl(7);

    expect(result).toEqual({ status: "no-url" });
    expect(message).toHaveBeenCalledWith("No openable page for this result.", {
      title: "Couldn't open page",
      kind: "info",
    });
  });

  test("error (string): invoke throws a string -> {status:'error',error:<string>} + error dialog", async () => {
    invoke.mockImplementation(async () => {
      throw "broker exploded";
    });

    const result = await openCapturedUrl(7);

    expect(result).toEqual({ status: "error", error: "broker exploded" });
    expect(message).toHaveBeenCalledWith("Couldn't open URL: broker exploded", {
      title: "Couldn't open page",
      kind: "error",
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
    expect(message).toHaveBeenCalledWith(
      "Couldn't open URL: the page could not be opened",
      { title: "Couldn't open page", kind: "error" },
    );
  });
});

describe("openCapturedUrl silent mode (dashboard contract)", () => {
  test("silent suppresses the no-url dialog but still returns the status", async () => {
    invoke.mockImplementation(async () => false);

    const result = await openCapturedUrl(7, { silent: true });

    expect(result).toEqual({ status: "no-url" });
    expect(message).not.toHaveBeenCalled();
  });

  test("silent suppresses the error dialog but still returns status + error", async () => {
    invoke.mockImplementation(async () => {
      throw "kaboom";
    });

    const result = await openCapturedUrl(7, { silent: true });

    expect(result).toEqual({ status: "error", error: "kaboom" });
    expect(message).not.toHaveBeenCalled();
  });
});
