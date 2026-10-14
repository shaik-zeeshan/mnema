// @ts-nocheck — exercised by `bun test`; see audio-drawer-view.test.ts (that file
// is over the 800-line cap, so the speaker-strip checks live here).
import { describe, expect, it } from "bun:test";
import { assignSpeakerMarks, buildSpeakerGroups, speakerStrip } from "./audio-drawer-view";

const turn = (id, clusterId, startMs, endMs, over = {}) => ({
  id,
  clusterId,
  speakerLabel: `Unknown Speaker ${clusterId}`,
  personId: null,
  suggestedPersonId: null,
  startMs,
  endMs,
  transcriptText: "words",
  overlaps: false,
  ...over,
});

const PROFILES = [
  { id: 1, displayName: "Shaik Zeeshan", isAccountOwner: true },
  { id: 2, displayName: "Daniel Okafor", isAccountOwner: false },
];

describe("assignSpeakerMarks owner pin", () => {
  it("pins the owner cluster to communication without spending a palette slot", () => {
    const marks = assignSpeakerMarks([12, 11, 13], new Set([11]));
    expect(marks.get(11).colorVar).toBe("--cat-communication");
    expect(marks.get(12).colorVar).toBe("--cat-meetings");
    expect(marks.get(13).colorVar).toBe("--cat-research");
    expect(marks.get(11).shape).toBe("square"); // shape still by first appearance
  });
});

describe("speakerStrip", () => {
  const turns = [
    turn(1, 12, 0, 3000),
    turn(2, 11, 3000, 4000, { personId: 1 }),
    turn(3, 13, 4000, 8000, { suggestedPersonId: 2 }),
    turn(4, 14, 8000, 10000, { transcriptText: null }), // wordless: no chip, no share
  ];
  const clusters = [{ id: 11, personLinkAuto: true }];
  const strip = speakerStrip(buildSpeakerGroups(turns, clusters), turns, clusters, PROFILES);

  it("lists each speaker once with owner / suggestion / unnamed state and talk share", () => {
    expect(strip.map((s) => [s.clusterId, s.name, s.state, s.sharePct])).toEqual([
      [12, "Unknown Speaker 12", "unnamed", 38],
      [11, "Shaik Zeeshan", "you · auto", 13],
      [13, "Unknown Speaker 13", "maybe Daniel", 50],
    ]);
    expect(strip.filter((s) => s.owner).map((s) => s.clusterId)).toEqual([11]);
  });
});
