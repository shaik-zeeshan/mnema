// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import type {
  Activity,
  Conclusion,
  ConclusionEvidenceRef,
  SubjectTrajectory,
} from "$lib/types/recording";
import {
  buildTimeline,
  chartPoints,
  chartScope,
  chartX,
  interpolateConfidence,
  type TimelineEvent,
} from "./subjectTimeline";

function conclusion(over: Partial<Conclusion> = {}): Conclusion {
  return {
    id: 1,
    subject: "s",
    statement: "stmt",
    confidence: 0.5,
    status: "visible",
    pinned: false,
    formedAtMs: 1_000,
    lastSupportedAtMs: 5_000,
    updatedAtMs: 5_000,
    evidence: [],
    ...over,
  };
}

function evidenceRef(over: Partial<ConclusionEvidenceRef> = {}): ConclusionEvidenceRef {
  return { activityId: 10, stance: "support", ...over };
}

function activity(over: Partial<Activity> = {}): Activity {
  return {
    id: 10,
    title: "Wrote code",
    summary: "",
    startedAtMs: 2_000,
    endedAtMs: 3_000,
    createdAtMs: 3_000,
    evidence: [],
    ...over,
  };
}

function trajectory(history: [number, number][]): SubjectTrajectory {
  return {
    conclusionId: 1,
    statement: "stmt",
    history: history.map(([confidence, snapshotAtMs]) => ({ confidence, snapshotAtMs })),
  };
}

describe("buildTimeline", () => {
  test("merges evidence + markers newest-first, formed last", () => {
    const c = conclusion({
      formedAtMs: 500,
      evidence: [evidenceRef({ activityId: 10 })],
    });
    const acts = new Map<number, Activity>([[10, activity({ startedAtMs: 3_000 })]]);
    const traj = trajectory([
      [0.3, 1_000],
      [0.9, 4_000], // +0.6 => reinforced marker @4000
    ]);
    const events = buildTimeline(c, traj, acts);

    const kinds = events.map((e) => e.kind);
    expect(kinds[kinds.length - 1]).toBe("formed");
    // atMs of non-formed events must be descending.
    const ats = events
      .filter((e) => e.kind !== "formed")
      .map((e) => (e as { atMs: number | null }).atMs);
    for (let i = 1; i < ats.length; i++) {
      expect((ats[i - 1] ?? -Infinity) >= (ats[i] ?? -Infinity)).toBe(true);
    }
    // marker @4000 comes before evidence @3000.
    expect(events[0].kind).toBe("marker");
    expect(events[1].kind).toBe("evidence");
  });

  test("reinforced / decayed derive from correct snapshot deltas", () => {
    const traj = trajectory([
      [0.2, 100],
      [0.7, 200], // +0.5 reinforced
      [0.4, 300], // -0.3 decayed
    ]);
    const events = buildTimeline(conclusion(), traj, new Map());
    const markers = events.filter((e): e is Extract<TimelineEvent, { kind: "marker" }> => e.kind === "marker");
    expect(markers).toHaveLength(2);
    // Newest first: decayed @300, then reinforced @200.
    expect(markers[0]).toMatchObject({ direction: "decayed", from: 0.7, to: 0.4, atMs: 300 });
    expect(markers[1]).toMatchObject({ direction: "reinforced", from: 0.2, to: 0.7, atMs: 200 });
  });

  test("run of sub-0.04 same-direction steps collapses to one marker", () => {
    const traj = trajectory([
      [0.50, 100],
      [0.51, 200],
      [0.52, 300],
      [0.53, 400], // three +0.01 micro steps => one reinforced marker 0.50->0.53 @400
    ]);
    const markers = buildTimeline(conclusion(), traj, new Map()).filter(
      (e) => e.kind === "marker",
    );
    expect(markers).toHaveLength(1);
    expect(markers[0]).toMatchObject({ direction: "reinforced", from: 0.5, to: 0.53, atMs: 400 });
  });

  test("a >= 0.04 step stands alone (not collapsed)", () => {
    const traj = trajectory([
      [0.50, 100],
      [0.51, 200], // micro
      [0.60, 300], // +0.09 big step -> standalone
      [0.61, 400], // micro
    ]);
    const markers = buildTimeline(conclusion(), traj, new Map()).filter(
      (e) => e.kind === "marker",
    );
    // run(0.50->0.51) + big(0.51->0.60) + run(0.60->0.61) = 3 markers.
    expect(markers).toHaveLength(3);
    const big = markers.find((m) => (m as { from: number }).from === 0.51);
    expect(big).toMatchObject({ from: 0.51, to: 0.6, atMs: 300 });
  });

  test("contradict evidence ref renders as contradict, not evidence", () => {
    const c = conclusion({
      evidence: [
        evidenceRef({ activityId: 10, stance: "support" }),
        evidenceRef({ activityId: 11, stance: "contradict" }),
      ],
    });
    const events = buildTimeline(c, undefined, new Map());
    expect(events.some((e) => e.kind === "contradict")).toBe(true);
    const contra = events.find((e) => e.kind === "contradict");
    expect(contra).toMatchObject({ activityId: 11, kind: "contradict" });
    // The contradict ref is NOT also an evidence event.
    expect(events.filter((e) => e.kind === "evidence")).toHaveLength(1);
  });

  test("confidenceAt: formed=formation, marker=to, evidence=interpolated", () => {
    const c = conclusion({
      confidence: 0.9,
      formedAtMs: 100,
      evidence: [evidenceRef({ activityId: 10 })],
    });
    // Activity @150 sits between the two snapshots below.
    const acts = new Map<number, Activity>([[10, activity({ startedAtMs: 150 })]]);
    const traj = trajectory([
      [0.4, 100], // formation snapshot
      [0.8, 200], // +0.4 reinforced marker @200
    ]);
    const events = buildTimeline(c, traj, acts);

    const formed = events.find((e) => e.kind === "formed")!;
    expect(formed.confidenceAt).toBe(0.4); // first snapshot = formation

    const marker = events.find((e) => e.kind === "marker")!;
    expect(marker.confidenceAt).toBe(0.8); // the run's `to`

    const ev = events.find((e) => e.kind === "evidence")!;
    // Linear interp at t=(150-100)/(200-100)=0.5 → 0.4 + 0.5*0.4 = 0.6.
    expect(ev.confidenceAt).toBeCloseTo(0.6, 5);
  });

  test("evidence confidenceAt falls back to conclusion.confidence when no history / null atMs", () => {
    const c = conclusion({
      confidence: 0.73,
      evidence: [evidenceRef({ activityId: 10 })],
    });
    // No trajectory at all.
    const acts = new Map<number, Activity>([[10, activity({ startedAtMs: 500 })]]);
    const ev = buildTimeline(c, undefined, acts).find((e) => e.kind === "evidence")!;
    expect(ev.confidenceAt).toBe(0.73);
  });

  test("marker whose endpoints round to the same percent is dropped", () => {
    // 0.904 → 0.897 is a real -0.007 step (< MICRO_STEP so it collapses to a
    // run of one), but both round to 90% — a no-op the UI must not show.
    const traj = trajectory([
      [0.904, 100],
      [0.897, 200],
    ]);
    const markers = buildTimeline(conclusion(), traj, new Map()).filter(
      (e) => e.kind === "marker",
    );
    expect(markers).toHaveLength(0);
  });

  test("empty history => zero markers; evidence + formed still present", () => {
    const c = conclusion({ evidence: [evidenceRef()] });
    const acts = new Map<number, Activity>([[10, activity()]]);
    const events = buildTimeline(c, trajectory([[0.5, 100]]), acts);
    expect(events.filter((e) => e.kind === "marker")).toHaveLength(0);
    expect(events.filter((e) => e.kind === "evidence")).toHaveLength(1);
    expect(events.filter((e) => e.kind === "formed")).toHaveLength(1);
  });

  test("formed origin is the last element", () => {
    const c = conclusion({
      formedAtMs: 9_999_999, // even a far-future formedAt stays last
      evidence: [evidenceRef()],
    });
    const events = buildTimeline(c, trajectory([[0.3, 1], [0.9, 2]]), new Map());
    expect(events[events.length - 1].kind).toBe("formed");
  });

  test("replacedStatement + replacedAtMs => one 'replaced' event, sorted chronologically", () => {
    const c = conclusion({
      formedAtMs: 100,
      replacedStatement: "old wrong take",
      replacedAtMs: 3_000, // between the two markers below
      evidence: [],
    });
    const traj = trajectory([
      [0.3, 1_000],
      [0.9, 5_000], // reinforced marker @5000
    ]);
    const events = buildTimeline(c, traj, new Map());
    const replaced = events.filter((e) => e.kind === "replaced");
    expect(replaced).toHaveLength(1);
    expect(replaced[0]).toMatchObject({
      kind: "replaced",
      statement: "old wrong take",
      atMs: 3_000,
    });
    // Positioned by timestamp: after marker @5000, before formed origin.
    const idx = events.findIndex((e) => e.kind === "replaced");
    expect(events[idx - 1].kind).toBe("marker"); // @5000
    expect(events[events.length - 1].kind).toBe("formed"); // origin still last
  });

  test("no replacedStatement => no 'replaced' event", () => {
    const events = buildTimeline(conclusion(), undefined, new Map());
    expect(events.some((e) => e.kind === "replaced")).toBe(false);
    // replacedAtMs alone (no statement) also yields nothing.
    const partial = buildTimeline(conclusion({ replacedAtMs: 42 }), undefined, new Map());
    expect(partial.some((e) => e.kind === "replaced")).toBe(false);
  });

  test("evidence join: title/atMs/source fallbacks and frame source", () => {
    const c = conclusion({ evidence: [evidenceRef({ activityId: 10 })] });
    const acts = new Map<number, Activity>([
      [
        10,
        activity({
          title: "Reviewed PR",
          startedAtMs: 4_242,
          category: "creating",
          evidence: [{ subjectType: "frame", subjectId: 77, isHeadline: false }],
        }),
      ],
    ]);
    const [ev] = buildTimeline(c, undefined, acts);
    expect(ev).toMatchObject({
      kind: "evidence",
      title: "Reviewed PR",
      atMs: 4_242,
      category: "creating",
      sourceType: "screen",
      frameId: 77,
    });

    // Unresolved activity => ref fallbacks + null source.
    const c2 = conclusion({
      evidence: [
        evidenceRef({ activityId: 99, activityTitle: "Ghost", activityStartedAtMs: 88 }),
      ],
    });
    const [ev2] = buildTimeline(c2, undefined, new Map());
    expect(ev2).toMatchObject({ title: "Ghost", atMs: 88, sourceType: null, frameId: null });

    // Audio source from first ref.
    const c3 = conclusion({ evidence: [evidenceRef({ activityId: 10 })] });
    const acts3 = new Map<number, Activity>([
      [10, activity({ evidence: [{ subjectType: "audio_segment", subjectId: 5, isHeadline: false }] })],
    ]);
    const [ev3] = buildTimeline(c3, undefined, acts3);
    expect(ev3).toMatchObject({ sourceType: "audio", frameId: null });
  });
});

describe("chartPoints", () => {
  test("time axis: domain start → 0, last snapshot → right edge, evidence clamps, one snapshot flat to now", () => {
    const h = trajectory([[0.4, 1_000], [0.6, 2_000], [0.8, 3_000]]).history;
    // Domain [1000, 3000]: first snapshot sits at x 0, the last at the right edge.
    expect(chartPoints(h, 1_000, 3_000)).toEqual([{ x: 0, y: 0.4 }, { x: 0.5, y: 0.6 }, { x: 1, y: 0.8 }]);
    // An evidence time before the first retained snapshot clamps to its confidence.
    expect(chartX(500, 1_000, 3_000)).toBe(0);
    expect(interpolateConfidence(h, 500, 0.9)).toBe(0.4);
    // A one-snapshot history draws flat from that snapshot to now.
    expect(chartPoints([{ confidence: 0.5, snapshotAtMs: 2_000 }], 1_000, 5_000)).toEqual([
      { x: 0.25, y: 0.5 },
      { x: 1, y: 0.5 },
    ]);
    expect(chartPoints([], 1_000, 5_000)).toEqual([]);
  });
});

describe("chartScope", () => {
  const D = 86_400_000;
  test("draws the top N plus the selected, on an axis fitted to their data", () => {
    const cs = [1, 2, 3, 4].map((id) => conclusion({ id, formedAtMs: id * 10 * D, lastSupportedAtMs: id * 10 * D, evidence: [] }));
    const histories = new Map([[2, [{ snapshotAtMs: 15 * D, confidence: 0.4 }, { snapshotAtMs: 25 * D, confidence: 0.5 }]]]);
    const scope = chartScope(cs, histories, 4, 2);
    expect(scope.lines.map((l) => l.conclusion.id)).toEqual([1, 2, 4]);
    expect([scope.fromMs, scope.toMs]).toEqual([10 * D, 40 * D]);
    // One point in time still spans a day.
    expect(chartScope(cs.slice(0, 1), new Map(), 1)).toMatchObject({ fromMs: 9 * D, toMs: 10 * D });
  });
});
