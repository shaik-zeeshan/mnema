<script lang="ts">
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

<!-- The mounted tab panel for the `intelligence` toolbar tab. Its groups lay out on
     the bento grid (settings-bento.css). -->
<div
  class="settings-panel"
  id="settings-panel-intelligence"
  role="tabpanel"
  aria-labelledby="settings-tab-intelligence"
>
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
