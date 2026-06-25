<script lang="ts">
  import Providers from "./Providers.svelte";
  import AskAi from "./AskAi.svelte";
  import UserContext from "./UserContext.svelte";
  import Ocr from "./Ocr.svelte";
  import Transcription from "./Transcription.svelte";
  import Speakers from "./Speakers.svelte";
  import SemanticSearch from "./SemanticSearch.svelte";
</script>

<div role="tabpanel" id="settings-panel-intelligence" tabindex="0">
  <Providers />
  <AskAi />
  <UserContext />
  <Ocr />
  <Transcription />
  <Speakers />
  <SemanticSearch />
</div>
