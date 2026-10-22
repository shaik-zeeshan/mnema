// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { expect, test } from "bun:test";
import { permissionRowState } from "./permission-rows";

test("permission row offers the prompt only until macOS has an answer", () => {
  expect(permissionRowState("granted")).toBe("granted");
  expect(permissionRowState("not_determined")).toBe("ask");
  expect(permissionRowState("unknown")).toBe("ask");
  expect(permissionRowState(undefined)).toBe("ask");
  expect(permissionRowState("denied")).toBe("denied");
  expect(permissionRowState("restricted")).toBe("denied");
  expect(permissionRowState("unsupported")).toBe("hidden");
});
