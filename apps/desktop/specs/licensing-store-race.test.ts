// Race regression: the boot snapshot (`get_license_status`) and the live
// `license_status` event travel over independent IPC timing, and the ADR 0055
// heal path emits a fresh event proactively at startup. A late-resolving stale
// snapshot must never clobber the fresher event value — the observable failure
// is a healed (activated) machine snapping back to "lapsed" and nagging the
// user to reconnect.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles
// the REAL store with Svelte's compiler under node (same harness as
// jumper-reactivity.test.ts).
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");

// Deferred snapshot + captured event callback so the test can force the
// adversarial order: the FRESH event lands first, THEN the stale snapshot
// promise resolves.
let resolveSnapshot: (v: unknown) => void = () => {};
const snapshot = new Promise((r) => {
	resolveSnapshot = r;
});
let eventCb: ((e: { payload: unknown }) => void) | null = null;

mock.module("@tauri-apps/api/core", () => ({
	invoke: async (cmd: string) => (cmd === "get_license_status" ? snapshot : undefined),
	// bun module mocks fix the export-name set process-wide; keep parity with
	// the other specs' mock of this module.
	convertFileSrc: (p: string) => p,
}));
mock.module("@tauri-apps/api/event", () => ({
	listen: async (_name: string, cb: (e: { payload: unknown }) => void) => {
		eventCb = cb;
		return () => {};
	},
}));

// initLicenseStatus no-ops without a window (SSR guard).
(globalThis as { window?: unknown }).window ??= {};

beforeAll(() => {
	const built = spawnSync("node", [resolve(here, "build.mjs")], { stdio: "inherit" });
	if (built.status !== 0) throw new Error("precompile failed");
});

const STALE = {
	kind: "licensed",
	updateThroughMs: 1,
	inWindow: true,
	email: "a@b.c",
	name: "",
	activation: { state: "lapsed" },
};
const FRESH = {
	kind: "licensed",
	updateThroughMs: 1,
	inWindow: true,
	email: "a@b.c",
	name: "",
	activation: { state: "activated" },
};

type StoreModule = {
	initLicenseStatus: () => void;
	licenseStatus: { value: { activation: { state: string } } | null };
};

test("fresh license_status event is not clobbered by a late stale snapshot", async () => {
	const { initLicenseStatus, licenseStatus } = (await import(
		"./_reactivity/gen/licensing-store.js"
	)) as StoreModule;

	initLicenseStatus();
	expect(eventCb).toBeTruthy();

	// Fresh event arrives first (receipt-refresh heal, ADR 0055).
	eventCb?.({ payload: FRESH });
	expect(licenseStatus.value?.activation.state).toBe("activated");

	// Now the stale get_license_status snapshot resolves late. Drain with a
	// macrotask so the store's .then (behind async-mock promise adoption) has
	// definitely run before asserting.
	resolveSnapshot(STALE);
	await snapshot;
	await new Promise((r) => setTimeout(r, 0));

	// Must still be the fresh state, never regressed to the stale snapshot.
	expect(licenseStatus.value?.activation.state).toBe("activated");
});
