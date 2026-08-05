<script lang="ts">
  import Storage from "./Storage.svelte";
  import Access from "./Access.svelte";
</script>

<!-- The mounted tab panel for the `data` toolbar tab. Its groups lay out on
     the bento grid (settings-bento.css). -->
<div
  class="settings-panel"
  id="settings-panel-data"
  role="tabpanel"
  aria-labelledby="settings-tab-data"
>
  <Storage />
  <Access />
</div>
