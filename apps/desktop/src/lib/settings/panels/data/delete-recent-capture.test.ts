// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
  DELETE_WINDOW_OPTIONS,
  deleteConfirmMessage,
  deleteReceipt,
  deleteWindowPhrase,
} from "./delete-recent-capture";

describe("delete recent capture copy", () => {
  test("offers exactly the three windows the tray offers", () => {
    expect(DELETE_WINDOW_OPTIONS).toEqual([
      { value: "60", label: "1 min" },
      { value: "300", label: "5 min" },
      { value: "900", label: "15 min" },
    ]);
  });

  test("phrases match the tray (singular minute)", () => {
    expect(deleteWindowPhrase(60)).toBe("last 1 minute");
    expect(deleteWindowPhrase(300)).toBe("last 5 minutes");
    expect(deleteConfirmMessage(900)).toBe(
      "Delete the last 15 minutes from Mnema's library? This removes whole overlapping capture segments and cannot be undone.",
    );
  });

  test("receipt is one line, warning only when files were left behind", () => {
    expect(deleteReceipt(300, 0)).toEqual({ tone: "ok", text: "Deleted the last 5 minutes." });
    expect(deleteReceipt(60, 1)).toEqual({
      tone: "warn",
      text: "Deleted the last 1 minute, but 1 file couldn’t be removed from disk yet — Mnema will retry.",
    });
  });
});
