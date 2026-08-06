<script lang="ts">
  import PanelHeading from "$lib/settings/ui/PanelHeading.svelte";
  import Capture from "./Capture.svelte";
  import Video from "./Video.svelte";
  import Audio from "./Audio.svelte";
  import Privacy from "./Privacy.svelte";
</script>

<div class="settings-panel" id="settings-panel-capture">
  <!-- No "N MB today" figure here: nothing measures a per-DAY total, only a
       rolling average, and the Video Output row already states that average
       against the slider that changes it (G8). -->
  <PanelHeading title="Capture" meta="What is recorded, how it is encoded, and what is never recorded at all" />
  <Capture />
  <Video />
  <Audio />
  <Privacy />
</div>
