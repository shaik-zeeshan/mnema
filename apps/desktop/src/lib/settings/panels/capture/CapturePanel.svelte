<script lang="ts">
  import Capture from "./Capture.svelte";
  import Video from "./Video.svelte";
  import Audio from "./Audio.svelte";
  import Privacy from "./Privacy.svelte";
</script>

<div role="tabpanel" id="settings-panel-capture" tabindex="0">
  <Capture />
  <Video />
  <Audio />
  <Privacy />
</div>
