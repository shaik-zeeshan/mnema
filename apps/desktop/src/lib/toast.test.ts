// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import {
	TOAST_AUTO_DISMISS_MS,
	TOAST_VISIBLE_MAX,
	autoDismissDelayMs,
	coalesceNewest,
	overflowCount,
	pushToastCore,
	removeToastCore,
	visibleToasts,
} from "./toast";

function makeStore() {
	let id = 1;
	const stack = [];
	const archive = [];
	const push = (kind, message, options = {}, nowMs = 1000) =>
		pushToastCore(stack, archive, () => id++, kind, message, options, nowMs);
	return { stack, archive, push };
}

describe("toast stack cap and overflow", () => {
	test("up to three toasts are all visible with no overflow", () => {
		const { stack, push } = makeStore();
		push("info", "one");
		push("info", "two");
		push("info", "three");
		expect(visibleToasts(stack).map((t) => t.message)).toEqual(["one", "two", "three"]);
		expect(overflowCount(stack)).toBe(0);
	});

	test("a fourth toast collapses the oldest into +N overflow", () => {
		const { stack, push } = makeStore();
		push("info", "one");
		push("info", "two");
		push("info", "three");
		push("danger", "four");
		expect(visibleToasts(stack)).toHaveLength(TOAST_VISIBLE_MAX);
		// Newest wins a visible slot; the oldest is the one collapsed.
		expect(visibleToasts(stack).map((t) => t.message)).toEqual(["two", "three", "four"]);
		expect(overflowCount(stack)).toBe(1);
		expect(overflowCount(stack.slice(0, 2))).toBe(0);
	});
});

describe("sticky vs auto-dismiss", () => {
	test("danger toasts never auto-dismiss", () => {
		expect(autoDismissDelayMs("danger")).toBeNull();
	});

	test("info and success auto-dismiss after ~6s", () => {
		expect(autoDismissDelayMs("info")).toBe(TOAST_AUTO_DISMISS_MS);
		expect(autoDismissDelayMs("success")).toBe(TOAST_AUTO_DISMISS_MS);
		expect(TOAST_AUTO_DISMISS_MS).toBe(6000);
	});
});

describe("coalesce consecutive identical", () => {
	test("an identical repeat bumps the newest entry instead of stacking", () => {
		const { stack, archive, push } = makeStore();
		const first = push("danger", "OCR failed", { detail: "attempt 1" }, 1000);
		const second = push("danger", "OCR failed", { detail: "attempt 2" }, 2000);
		expect(second.id).toBe(first.id);
		expect(stack).toHaveLength(1);
		expect(archive).toHaveLength(1);
		expect(stack[0].count).toBe(2);
		expect(stack[0].detail).toBe("attempt 2");
		expect(stack[0].createdAtUnixMs).toBe(2000);
	});

	test("a different message or kind does not coalesce", () => {
		const { stack, push } = makeStore();
		push("danger", "OCR failed");
		push("danger", "Audio lane unavailable");
		push("info", "Audio lane unavailable");
		expect(stack).toHaveLength(3);
	});

	test("only the NEWEST entry coalesces — an interleaved toast breaks the run", () => {
		const { stack, push } = makeStore();
		push("danger", "OCR failed");
		push("info", "Export finished");
		push("danger", "OCR failed");
		expect(stack).toHaveLength(3);
		expect(coalesceNewest(stack, "info", "Export finished", null, 0)).toBeNull();
	});
});

describe("archive", () => {
	test("every push is archived; dismissing the toast keeps the record", () => {
		const { stack, archive, push } = makeStore();
		const toast = push("success", "Export finished");
		expect(archive).toHaveLength(1);
		expect(removeToastCore(stack, toast.id)).toBe(true);
		expect(stack).toHaveLength(0);
		expect(archive).toHaveLength(1);
		expect(archive[0].message).toBe("Export finished");
	});

	test("archive entries are independent copies (no aliasing with the stack)", () => {
		const { stack, archive, push } = makeStore();
		push("info", "one");
		expect(archive[0]).not.toBe(stack[0]);
		expect(archive[0].id).not.toBe(stack[0].id);
	});

	test("removeToastCore on an unknown id is a no-op", () => {
		const { stack, push } = makeStore();
		push("info", "one");
		expect(removeToastCore(stack, 999)).toBe(false);
		expect(stack).toHaveLength(1);
	});
});
