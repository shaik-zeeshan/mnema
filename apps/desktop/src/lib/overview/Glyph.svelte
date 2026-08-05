<script lang="ts">
  // The handful of 14×14 glyphs the Overview's tiles need, drawn from the
  // direction mockup's sprite. The app has no SVG sprite sheet, so this is the
  // one component that carries the paths; every consumer sizes it with CSS.
  export type GlyphName = "chevr" | "screen" | "mic" | "sys" | "spark-o" | "scan";

  let { name, class: className = "" }: { name: GlyphName; class?: string } = $props();
</script>

<svg
  class={className}
  viewBox={name === "chevr" ? "0 0 8 12" : "0 0 14 14"}
  fill="none"
  aria-hidden="true"
>
  {#if name === "chevr"}
    <path
      d="M2 1.5 6.5 6 2 10.5"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  {:else if name === "screen"}
    <rect x="1.5" y="2.5" width="11" height="7.5" rx="1.4" stroke="currentColor" stroke-width="1.3" />
    <path d="M5 12h4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
  {:else if name === "mic"}
    <rect x="5" y="1.5" width="4" height="7" rx="2" stroke="currentColor" stroke-width="1.3" />
    <path
      d="M3 7a4 4 0 0 0 8 0M7 11v1.8"
      stroke="currentColor"
      stroke-width="1.3"
      stroke-linecap="round"
    />
  {:else if name === "sys"}
    <path
      d="M2.5 5.5h2.4L8 2.8v8.4L4.9 8.5H2.5z"
      stroke="currentColor"
      stroke-width="1.3"
      stroke-linejoin="round"
    />
    <path d="M10 5a3 3 0 0 1 0 4" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
  {:else if name === "spark-o"}
    <path
      d="M7 1.8 8.3 5.7 12.2 7 8.3 8.3 7 12.2 5.7 8.3 1.8 7 5.7 5.7z"
      stroke="currentColor"
      stroke-width="1.3"
      stroke-linejoin="round"
    />
  {:else if name === "scan"}
    <path
      d="M1.5 4V2.5A1 1 0 0 1 2.5 1.5H4M10 1.5h1.5a1 1 0 0 1 1 1V4M12.5 10v1.5a1 1 0 0 1-1 1H10M4 12.5H2.5a1 1 0 0 1-1-1V10"
      stroke="currentColor"
      stroke-width="1.3"
      stroke-linecap="round"
    />
    <path d="M4.2 5.5h5.6M4.2 8.5h3.6" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" />
  {/if}
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
