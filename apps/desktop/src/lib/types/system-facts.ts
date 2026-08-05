// Hand-mirrored wire type for `crates/capture-types/src/system_facts.rs`
// (`get_system_facts`). Keep the two in sync — the Rust side carries the
// serde round-trip test.
//
// Round-4 decision G8: every field is nullable because a denominator ships only
// where the value is real on this machine. `null` means "render no number",
// never "render zero".
export interface SystemFacts {
	/** Capture root every disk figure was measured against. */
	capturePath: string;
	diskFreeBytes: number | null;
	totalRamBytes: number | null;
	/** Measured average over `measuredDays` complete capture days. */
	measuredBytesPerDay: number | null;
	measuredDays: number;
	/** The configured screen capture rate, for projecting the measured average. */
	screenFrameRate: number | null;
	ocrBacklog: number | null;
	transcriptionBacklog: number | null;
	semanticVectorCount: number | null;
	semanticPendingCount: number | null;
	/** Bytes of one stored vector — `int8[768]`, a schema fact. */
	semanticVectorBytes: number;
	databaseBytes: number | null;
}
