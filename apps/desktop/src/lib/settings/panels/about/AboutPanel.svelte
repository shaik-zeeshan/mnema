<script lang="ts">
  import About from "./About.svelte";
  import License from "./License.svelte";
  import Developer from "./Developer.svelte";
</script>

<!-- The mounted tab panel for the `about` toolbar tab. Its groups lay out on
     the bento grid (settings-bento.css). -->
<div
  class="settings-panel"
  id="settings-panel-about"
  role="tabpanel"
  aria-labelledby="settings-tab-about"
>
  <License />
  <About />
  <Developer />
</div>
