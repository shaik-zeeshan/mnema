<script lang="ts">
  import Capture from "./Capture.svelte";
  import Video from "./Video.svelte";
  import Audio from "./Audio.svelte";
  import Privacy from "./Privacy.svelte";
</script>

<!-- The mounted tab panel for the `capture` toolbar tab. Its groups lay out on
     the bento grid (settings-bento.css). -->
<div
  class="settings-panel"
  id="settings-panel-capture"
  role="tabpanel"
  aria-labelledby="settings-tab-capture"
>
  <Capture />
  <Video />
  <Audio />
  <Privacy />
</div>
