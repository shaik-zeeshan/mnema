<script lang="ts">
  import PanelHeading from "$lib/settings/ui/PanelHeading.svelte";
  import Providers from "./Providers.svelte";
  import McpConnectors from "./McpConnectors.svelte";
  import AskAi from "./AskAi.svelte";
  import UserContext from "./UserContext.svelte";
  import Ocr from "./Ocr.svelte";
  import Transcription from "./Transcription.svelte";
  import Speakers from "./Speakers.svelte";
  import VoiceEnrollment from "./VoiceEnrollment.svelte";
  import SemanticSearch from "./SemanticSearch.svelte";
</script>

<div class="settings-panel" id="settings-panel-intelligence">
  <PanelHeading
    title="Intelligence"
    meta="Providers, Ask AI, understanding, and the four on-device model families"
  />
  <Providers />
  <McpConnectors />
  <AskAi />
  <UserContext />
  <Ocr />
  <Transcription />
  <Speakers />
  <!-- Voice enrollment stacks under Speakers: it is the same section (one rail
       entry), and the read-out only makes sense beside the two toggles it
       reports on. -->
  <VoiceEnrollment />
  <SemanticSearch />
</div>
