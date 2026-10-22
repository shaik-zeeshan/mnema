<script lang="ts">
  // Inline busy spinner shown beside a button label while an action is in
  // flight (download / cancel / delete). A spinning Lucide loader (the kit's
  // global `mx-spin` keyframes); the label beside it carries the state.
  //
  // Extracted from the `.btn-spinner` svg every settings panel used to inline
  // and duplicate. The spinner svg is rendered by a child (icon) component, so
  // its styles live on the wrapper + a `:global(svg)` descendant rule — a class
  // passed to the icon component would NOT pick up this component's scope hash.
  import IconSpinner from "~icons/lucide/loader-circle";
</script>

<span class="btn-spinner" aria-hidden="true"><IconSpinner /></span>

<style>
  .btn-spinner {
    display: inline-flex;
    margin-right: 6px;
    vertical-align: -2px;
    animation: mx-spin 0.8s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .btn-spinner { animation: none; }
  }
  .btn-spinner :global(svg) {
    width: 13px;
    height: 13px;
    stroke-width: 2;
  }
</style>
