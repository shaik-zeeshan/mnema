<script lang="ts">
  import Providers from "./Providers.svelte";
  import McpConnectors from "./McpConnectors.svelte";
  import AskAi from "./AskAi.svelte";
  import UserContext from "./UserContext.svelte";
  import Ocr from "./Ocr.svelte";
  import Transcription from "./Transcription.svelte";
  import Speakers from "./Speakers.svelte";
  import GpuAcceleration from "./GpuAcceleration.svelte";
  import SemanticSearch from "./SemanticSearch.svelte";
</script>

<div class="settings-panel" id="settings-panel-intelligence">
  <Providers />
  <McpConnectors />
  <AskAi />
  <UserContext />
  <Ocr />
  <Transcription />
  <Speakers />
  <!-- Windows-only: self-gates on `detectKeyboardPlatform()`, rendering nothing on
       macOS so the Settings experience there is unchanged (rail also hides it). -->
  <GpuAcceleration />
  <SemanticSearch />
</div>
