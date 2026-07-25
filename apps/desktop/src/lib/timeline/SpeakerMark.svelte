<script lang="ts">
  // The dual-encoded speaker marker: colour (derived from clusterId, shared with
  // the Activity Receipt palette) AND one of a four-shape cycle. The shape is an
  // accessibility requirement, not decoration — colour alone dies in greyscale,
  // at low contrast, and under any form of colour blindness.
  //
  // 12px, not 9px: at 9px a triangle, a diamond and a square are the same grey
  // speck, which defeats the entire point (mockup AUDIT 11).
  import type { SpeakerMark } from "./audio-drawer-view";

  interface Props {
    mark: SpeakerMark | undefined;
    /** Hollow/hatched: this voice has no name yet. */
    ghosted?: boolean;
  }

  let { mark, ghosted = false }: Props = $props();
</script>

<span
  class="mark mark--{mark?.shape ?? 'circle'}"
  class:mark--ghosted={ghosted}
  style={mark?.colorVar ? `--sp: var(${mark.colorVar});` : ""}
  aria-hidden="true"
></span>

<style>
  .mark {
    width: 12px;
    height: 12px;
    flex: none;
    background: var(--sp, var(--app-text-subtle));
  }

  .mark--circle {
    border-radius: 999px;
  }

  .mark--square {
    border-radius: 1px;
  }

  .mark--triangle {
    clip-path: polygon(50% 0, 100% 100%, 0 100%);
  }

  .mark--diamond {
    clip-path: polygon(50% 0, 100% 50%, 50% 100%, 0 50%);
  }

  /* Unnamed = visibly not-yet-named, never red, never a warning. A dashed
     outline reads as "hollow" for circle/square; clip-path eats a border, so
     triangle/diamond get a hatch fill instead. */
  .mark--ghosted {
    background: transparent;
    border: 2px dashed var(--sp, var(--app-text-subtle));
  }

  .mark--triangle.mark--ghosted,
  .mark--diamond.mark--ghosted {
    border: none;
    background: repeating-linear-gradient(
      45deg,
      var(--sp, var(--app-text-subtle)) 0 1px,
      transparent 1px 4px
    );
  }
</style>
