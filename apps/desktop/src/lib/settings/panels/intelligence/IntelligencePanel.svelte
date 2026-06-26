<script lang="ts">
  import Providers from "./Providers.svelte";
  import AskAi from "./AskAi.svelte";
  import UserContext from "./UserContext.svelte";
  import Ocr from "./Ocr.svelte";
  import Transcription from "./Transcription.svelte";
  import Speakers from "./Speakers.svelte";
  import GpuAcceleration from "./GpuAcceleration.svelte";
  import SemanticSearch from "./SemanticSearch.svelte";
</script>

<div role="tabpanel" id="settings-panel-intelligence" tabindex="0">
  <Providers />
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
