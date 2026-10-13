// "Copy diagnostics" (the strip's ⋯ menu, ⇧⌘C): one plain-text block a user can
// paste into a bug report — app version, the health rows, per-processor queues,
// the meaning-index counts and the last 200 lines of rust.log.
//
// Never add the privacy snapshot (window titles, URLs) or the excluded-app list:
// this text leaves the machine.

import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import type { AppLogTailDto, ProcessorPipelineStatus, SemanticIndexStatusDto } from "$lib/types";

export const DIAGNOSTICS_LOG_LINES = 200;

/** Structural, so it compiles before and after B1 widens `FeatureHealthDto`. */
export interface DiagnosticsHealthRow {
	feature: string;
	severity: string;
	reason: string;
	metric?: string | null;
}

export interface DiagnosticsInput {
	version: string | null;
	generatedAtMs: number;
	health: DiagnosticsHealthRow[];
	pipeline: ProcessorPipelineStatus[] | null;
	semantic: SemanticIndexStatusDto | null;
	log: AppLogTailDto | null;
}

export function formatDiagnostics(d: DiagnosticsInput): string {
	const out = [
		"Mnema diagnostics",
		`version: ${d.version ?? "unknown"}`,
		`generated: ${new Date(d.generatedAtMs).toISOString()}`,
		"",
		"## Health",
	];
	if (d.health.length === 0) out.push("(not loaded)");
	for (const r of d.health) {
		out.push(`${r.feature}  ${r.severity}  ${r.reason}${r.metric ? `  [${r.metric}]` : ""}`);
	}

	out.push("", "## Queues");
	if (!d.pipeline) out.push("(unavailable)");
	else if (d.pipeline.length === 0) out.push("(none)");
	for (const p of d.pipeline ?? []) {
		const avg = p.averageCompletedSecondsLast24h == null ? "-" : `${p.averageCompletedSecondsLast24h.toFixed(1)}s`;
		out.push(
			`${p.processor}  queued ${p.queued} (retrying ${p.retrying})  running ${p.running}  done ${p.completed}  ` +
				`failed ${p.failed} (24h ${p.failedLast24h})  avg ${avg}${p.lastError ? `  last error: ${p.lastError}` : ""}`,
		);
	}
	if (d.semantic) {
		const s = d.semantic;
		out.push(
			`meaning index  vectors ${s.vectorCount}  backlog ${s.backlogCount}  dimension ${s.liveDimension ?? "none"}  ` +
				`model loaded ${s.modelLoaded ?? "-"}  load failures ${s.consecutiveLoadFailures ?? "-"}  ` +
				`quarantined ${s.quarantinedCount ?? "-"}${s.lastLoadError ? `  last load error: ${s.lastLoadError}` : ""}`,
		);
	}

	out.push("", `## Log (last ${DIAGNOSTICS_LOG_LINES} lines)`);
	if (!d.log) out.push("(unavailable)");
	else if (!d.log.exists) out.push(`(missing: ${d.log.path})`);
	else out.push(d.log.path, ...d.log.lines);
	return out.join("\n") + "\n";
}

/** Gather everything fresh; one failed source leaves its section "(unavailable)". */
export async function collectDiagnostics(health: DiagnosticsHealthRow[]): Promise<string> {
	const ok = <T>(r: PromiseSettledResult<T>): T | null => (r.status === "fulfilled" ? r.value : null);
	const [version, pipeline, semantic, log] = await Promise.allSettled([
		getVersion(),
		invoke<ProcessorPipelineStatus[]>("get_processing_pipeline_status"),
		invoke<SemanticIndexStatusDto>("get_semantic_index_status"),
		invoke<AppLogTailDto>("tail_app_log", { file: "rust", lines: DIAGNOSTICS_LOG_LINES }),
	]);
	return formatDiagnostics({
		version: ok(version),
		generatedAtMs: Date.now(),
		health,
		pipeline: ok(pipeline),
		semantic: ok(semantic),
		log: ok(log),
	});
}
