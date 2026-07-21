<script lang="ts">
  import Providers from "./Providers.svelte";
  import McpConnectors from "./McpConnectors.svelte";
  import AskAi from "./AskAi.svelte";
  import Triggers from "./Triggers.svelte";
  import UserContext from "./UserContext.svelte";
  import Ocr from "./Ocr.svelte";
  import Transcription from "./Transcription.svelte";
  import Speakers from "./Speakers.svelte";
  import SemanticSearch from "./SemanticSearch.svelte";
</script>

<div class="settings-panel" id="settings-panel-intelligence">
  <Providers />
  <McpConnectors />
  <AskAi />
  <Triggers />
  <UserContext />
  <Ocr />
  <Transcription />
  <Speakers />
  <SemanticSearch />
</div>
