<script lang="ts">
  // Logs — tail rust.log / native-capture-debug.log without leaving the app.
  //
  // The whole section is the shared <LogTail>; slice 7's per-feature "Log tail"
  // sub-tab mounts the same component with `feature` pre-selected.

  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import LogTail from "../LogTail.svelte";
  import { anchor } from "../sections";
</script>

<SettingGroup
  title="Logs"
  hint="rust.log · native-capture-debug.log"
  hintInline
  id={anchor("logs")}
>
  <div class="debug-body">
    <LogTail />
  </div>
</SettingGroup>
