// Pure (enabled, authMode, oauthState) → row-view mapping for the MCP connector
// list (Plan: MCP OAuth, slice 8a; mockup oauth-connectors.html `badge`/`sub`/
// `actions`/`is-off`). Kept free of runes and `invoke` so both the panel and the
// bun test import the SAME derivation — the badge/action rules live in one place.
//
// The enable toggle (Switch) is ALWAYS rendered by the component, independent of
// `authState`, so it is deliberately NOT part of `actions`.

import type { McpOAuthState } from "$lib/types";

export type McpRowBadge =
  | "secret"
  | "authorized"
  | "authorized-muted"
  | "not-connected"
  | "authorizing"
  | "reconnect"
  | "none";

export interface McpRowActions {
  /** OAuth, unauthorized: begin the browser flow. */
  connect: boolean;
  /** OAuth, token expired: begin the browser flow again. */
  reconnect: boolean;
  /** OAuth, authorized: revoke + drop the token. */
  disconnect: boolean;
  /** Open the edit/configure modal. */
  configure: boolean;
}

export interface McpRowView {
  badge: McpRowBadge;
  actions: McpRowActions;
  /** Dim the row (mockup `is-off`): a static/authorized connector switched off. */
  dimmed: boolean;
}

export interface McpRowInput {
  /** Draft auth mode; `undefined` (stdio, or a legacy http connector) ⇒ bearer. */
  authMode: "bearer" | "oauth" | undefined;
  enabled: boolean;
  /** Bearer: a static secret is present in the keychain. */
  hasSecret: boolean;
  /** OAuth: the lifecycle state; `undefined` (statuses not yet fetched) ⇒ none. */
  oauthState: McpOAuthState | undefined;
}

const NO_ACTIONS: McpRowActions = {
  connect: false,
  reconnect: false,
  disconnect: false,
  configure: false,
};

export function deriveMcpConnectorRow(input: McpRowInput): McpRowView {
  const { authMode, enabled, hasSecret, oauthState } = input;

  // Bearer (or unset ⇒ bearer): a static-token / optional-secret connector. The
  // only lifecycle is "is a secret saved"; Configure is the only extra action,
  // and the row dims when switched off.
  if (authMode !== "oauth") {
    return {
      badge: hasSecret ? "secret" : "none",
      actions: { ...NO_ACTIONS, configure: true },
      dimmed: !enabled,
    };
  }

  // OAuth: badge + actions follow the authorization lifecycle. `undefined`
  // (statuses not yet fetched, or a brand-new connector) reads as "none".
  switch (oauthState ?? "none") {
    case "authorized":
      // Only an authorized+enabled row offers Disconnect; disabled keeps the
      // token but dims (mockup) and drops Disconnect.
      return enabled
        ? {
            badge: "authorized",
            actions: { ...NO_ACTIONS, disconnect: true, configure: true },
            dimmed: false,
          }
        : {
            badge: "authorized-muted",
            actions: { ...NO_ACTIONS, configure: true },
            dimmed: true,
          };
    case "authorizing":
      // slice 8b: an in-modal cancel lands here; for now only the toggle shows.
      return { badge: "authorizing", actions: { ...NO_ACTIONS }, dimmed: false };
    case "reconnect":
      return {
        badge: "reconnect",
        actions: { ...NO_ACTIONS, reconnect: true, disconnect: true },
        dimmed: false,
      };
    case "none":
    default:
      // The crux row: enabled but not authorized. Never dimmed — it must be
      // impossible to miss that a Connect is owed.
      return { badge: "not-connected", actions: { ...NO_ACTIONS, connect: true }, dimmed: false };
  }
}
