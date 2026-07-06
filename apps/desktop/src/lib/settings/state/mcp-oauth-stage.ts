// Pure (live oauthState + local attempt flags) → connect-panel stage mapping for
// the in-modal OAuth connect flow (Plan: MCP OAuth, slice 8b; mockup
// oauth-connectors.html `oauthStageHTML`/`wireOauthStage`). Kept free of runes and
// `invoke` so both the picker and the bun test import the SAME derivation.
//
// The stage is DERIVED, not a timer (unlike the mockup's setTimeout fakes): the
// live `mcp_authorization_changed` → refresh → `mcpOAuthStateById[id]` change is
// what flips authorizing → authorized. The local flags disambiguate the two ways
// `state` can be non-authorized while a flow is in progress.

import type { McpOAuthState } from "$lib/types";

export type McpOAuthStage = "idle" | "authorizing" | "authorized" | "denied";

export interface McpOAuthStageInput {
  /** Live backend state for this connector; `undefined` before the id exists / is fetched. */
  state: McpOAuthState | undefined;
  /** The user clicked Connect at least once this modal session. */
  attempted: boolean;
  /** `beginMcpOAuth` recorded an error for this id (the browser never opened). */
  hasError: boolean;
  /** We have observed `state === "authorizing"` since the last attempt. */
  sawAuthorizing: boolean;
}

export function deriveMcpOAuthStage({
  state,
  attempted,
  hasError,
  sawAuthorizing,
}: McpOAuthStageInput): McpOAuthStage {
  // The token landed — the terminal happy state, regardless of local flags.
  if (state === "authorized") return "authorized";
  // Live backend authorizing always shows the browser-handoff stage.
  if (state === "authorizing") return "authorizing";
  // Nothing attempted yet (and not authorizing) → the resting Connect button.
  if (!attempted) return "idle";
  // begin threw (no DCR, network, …) → the browser never opened.
  if (hasError) return "denied";
  // attempted, no error, state ∈ {none, reconnect, undefined}:
  //   before we ever saw authorizing, begin is still in flight → optimistic wait;
  //   after we saw authorizing and state fell back, the browser round-trip was
  //   cancelled/denied (or the pending entry lapsed) → denied.
  return sawAuthorizing ? "denied" : "authorizing";
}

export interface McpOAuthPanelInput {
  /** Row Connect/Reconnect of an EXISTING connector (a `connectId` resolved a draft). */
  hasConnectServer: boolean;
  /** Modal step. */
  step: "catalog" | "connect";
  /** The selected preset is a hosted http+oauth preset (the catalog-add path). */
  isOAuthPreset: boolean;
  /** Edit/Configure mode of an existing connector. */
  edit: boolean;
}

// Whether to show the in-modal OAuth connect panel (McpOAuthConnect) instead of
// the normal add/edit form. Two doors open it: a row Connect/Reconnect
// (`hasConnectServer`), or step-2 of a catalog-added hosted-OAuth preset.
//
// Edit/Configure of an EXISTING oauth connector must NOT open the connect panel:
// edit mode carries no connect id (`oauthConnectId` is null), so the panel's
// Connect button would take the catalog-add branch and re-add a DUPLICATE
// connector — Configure has to route to the edit form (URL + Remove) instead.
export function showMcpOAuthConnectPanel({
  hasConnectServer,
  step,
  isOAuthPreset,
  edit,
}: McpOAuthPanelInput): boolean {
  if (hasConnectServer) return true;
  return step === "connect" && isOAuthPreset && !edit;
}
