// Viewport-anchored placement for the speaker-actions popover. Mirrors the
// CSS width `min(42rem, calc(100vw - 64px))` so the left clamp keeps the
// popover fully on-screen; anchored just above the clicked chip. Pure so the
// clamp branches are testable under `bun test` — the caller supplies the
// viewport and root font size.

export interface SpeakerPopoverPosition {
  left: number;
  bottom: number;
}

export function placeSpeakerActionsPopover(
  chip: { left: number; top: number },
  innerWidth: number,
  innerHeight: number,
  rem: number,
): SpeakerPopoverPosition {
  const width = Math.min(42 * rem, innerWidth - 64);
  return {
    left: Math.max(12, Math.min(chip.left, innerWidth - width - 12)),
    bottom: innerHeight - chip.top + 6,
  };
}
