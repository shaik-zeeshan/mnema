// One controller owning the debug page's stores, shared with every section via
// Svelte context. Mirrors the Settings shell's controller pattern
// (lib/settings/state/controller.svelte.ts).
//
// The shell (routes/debug/+page.svelte) builds it once and runs the mount /
// polling effects against it; sections read it via `getDebugController()`.

import { getContext, setContext } from "svelte";
import { createCaptureStore, type CaptureStore } from "./capture.svelte";
import { createPipelineStore, type PipelineStore } from "./pipeline.svelte";
import { createHealthStore, type HealthStore } from "./health.svelte";
import { createFeaturesStore, type FeaturesStore } from "./features.svelte";
import { createDetailStore, type DetailStore } from "./detail.svelte";

export type DebugController = {
	capture: CaptureStore;
	pipeline: PipelineStore;
	health: HealthStore;
	/** The five slice-6 feature cards' shared data (see features.svelte.ts). */
	features: FeaturesStore;
	/**
	 * The slice-7 drill-in: which feature detail is pushed (`null` = the summary
	 * scroll), and that view's own data. Nav state lives on the controller
	 * because the push comes from a section title deep in the tree while the
	 * shell is what renders summary-or-detail.
	 */
	detail: DetailStore;
};

export function createDebugController(): DebugController {
	return {
		capture: createCaptureStore(),
		pipeline: createPipelineStore(),
		health: createHealthStore(),
		features: createFeaturesStore(),
		detail: createDetailStore(),
	};
}

const KEY = Symbol("debug-controller");

export function setDebugController(controller: DebugController): void {
	setContext(KEY, controller);
}

export function getDebugController(): DebugController {
	const controller = getContext<DebugController | undefined>(KEY);
	if (!controller) throw new Error("DebugController missing — call setDebugController() in the debug shell");
	return controller;
}
