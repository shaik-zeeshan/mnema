<script lang="ts">
  // Inline busy spinner shown beside a button label while an action is in
  // flight (download / cancel / delete). A Lucide loader spun by the shared
  // `settings-icon-spin` keyframe (defined globally in settings-groups.css).
  //
  // Extracted from the `.btn-spinner` svg every settings panel used to inline
  // and duplicate. The spinner svg is rendered by a child (icon) component, so
  // its styles live on the wrapper + a `:global(svg)` descendant rule — a class
  // passed to the icon component would NOT pick up this component's scope hash.
  import IconSpinner from "~icons/lucide/loader-circle";
</script>

<span class="btn-spinner" aria-hidden="true"><IconSpinner /></span>

<style>
  /* Rotate the HTML wrapper, NOT the <svg> inside it: WKWebView does not reliably
     rotate an <svg> around its own center, so the icon would read as static. The
     span rotates around its box center as expected, and the icon rides along. */
  .btn-spinner {
    display: inline-flex;
    margin-right: 6px;
    vertical-align: -2px;
    animation: settings-icon-spin 0.7s linear infinite;
  }
  .btn-spinner :global(svg) {
    width: 13px;
    height: 13px;
    stroke-width: 2;
  }
  @media (prefers-reduced-motion: reduce) {
    .btn-spinner {
      animation: none;
    }
  }
</style>
