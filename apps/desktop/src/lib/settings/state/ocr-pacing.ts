/**
 * How hard OCR is allowed to work — the numbers behind the duty-cycle bar
 * (direction 04's `.duty`).
 *
 * These are NOT machine facts and NOT a setting: they are the shipped governor
 * constants from `apps/desktop/src-tauri/src/ocr_budget.rs`, whose own test
 * (`cooldown_holds_the_duty_cycle_in_both_bands`) pins them at ≤21% while
 * recording and ≤41% while paused. The bar reports the pacing Mnema actually
 * runs; it does not offer to change it.
 *
 * ponytail: mirrored, not wired — exposing them over IPC means a Rust change,
 * and this branch ships no Rust. If ocr_budget.rs retunes, move these with it.
 *
 * G8: no temperature and no ETA. There is no thermal sensor reading and no
 * measured OCR throughput anywhere in the app, so the bar states the split and
 * the real backlog and says nothing about heat or "done in N minutes" — the
 * mockup's "fans stay off" and "backlog clears in ≈ 22 min" are both dropped.
 */

export interface DutyCycle {
	/** What this pacing band is called in the UI. */
	label: string;
	/** Percent of each minute spent reading text (0–100). */
	workPercent: number;
	/** The rest of the minute (0–100). */
	coolPercent: number;
	/** The same split in seconds of a minute, for the prose line. */
	phrase: string;
}

function cycle(label: string, workPercent: number): DutyCycle {
	const workSeconds = Math.round((workPercent / 100) * 60);
	return {
		label,
		workPercent,
		coolPercent: 100 - workPercent,
		phrase: `${workSeconds} s of work, then ${60 - workSeconds} s idle, every minute`,
	};
}

/** While a recording is running, OCR takes a fifth of each minute. */
export const OCR_DUTY_RECORDING = cycle("While recording", 20);
/** Paused, it doubles to catch up on the backlog — still well under half. */
export const OCR_DUTY_PAUSED = cycle("While paused", 40);
