// Pure, framework-free upward-flip predicate shared by the inline Select and
// Combobox popovers. Both render inline (no body portal), so the panel can clip
// at the bottom of Settings' inner scroll container. On open each measures room
// below vs. above its trigger and flips upward when there isn't enough room
// below AND there's strictly more room above. Each caller passes its own
// `needed` constant (kept in sync with that component's content max-height).
//
// Conservative by design: the default is the existing downward open — we only
// flip up when below is genuinely tight and above is roomier.

export function shouldOpenUpward(
  spaceBelow: number,
  spaceAbove: number,
  needed: number,
): boolean {
  return spaceBelow < needed && spaceAbove > spaceBelow;
}
