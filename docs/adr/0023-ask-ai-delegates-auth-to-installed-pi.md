---
status: accepted
---

# Ask AI delegates provider auth to the user's installed PI

**Quick Recall**'s **Ask AI** delegates all LLM provider authentication to the user's already-installed, already-signed-in PI, reading its stored auth (`~/.pi/agent/auth.json`) as-is. Mnema collects no provider credentials, ships no API-key field, runs no OAuth flow, and operates no backend or token proxy. Mnema detects the installed PI the way **Access Settings** detects the `mnema` CLI and drives it over RPC (PI is Node-only); when PI is absent or has no configured provider, **Ask AI** is unavailable with a set-up-PI pointer.

This deliberately limits V1 **Ask AI** to users who have already set up PI. We investigated giving non-technical users an in-app "sign in with Claude/ChatGPT" experience, but PI exposes headless auth only for API keys (`AuthStorage.setRuntimeApiKey`) and no-auth local models; consumer OAuth subscription sign-in is bound to PI's interactive TUI `/login` with no SDK/GUI hook (only a partial Codex device-code path, [pi issue #2635](https://github.com/earendil-works/pi/issues/2635)). Building our own provider auth would reintroduce exactly the credential-handling and cloud-boundary surface that staying auth-free avoids.

**Considered Options**

We rejected an in-app API-key field (would make Mnema store a provider secret and still excludes non-technical users, who do not have API keys), bundling a PI/Node runtime with a GUI-hosted `/login` (heavy non-native dependency plus a fragile embedded-TUI auth flow), and a Mnema-operated proxy backend (contradicts [ADR 0022](0022-ask-ai-sends-redacted-capture-context-to-cloud-agent.md) and the no-backend posture). A local model via PI remains a valid no-auth provider choice the user can configure in PI, but it is not required by this decision.

**Consequences**

The accessible "sign in inside Mnema" path is intentionally deferred until PI offers headless OAuth; revisit this ADR when it does. Ask AI's reach is whatever the user's PI is configured for, and Mnema neither sees nor manages those credentials.
