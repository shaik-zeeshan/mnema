---
status: proposed
---

# AI features call models from the Rust side via rig-core, with a hybrid local/cloud engine and bring-your-own-key

Going forward, Mnema's AI features call language models **from the Rust side**, using
[`rig-core`](https://docs.rs/rig-core/) — a native Rust LLM/agent framework with multi-provider
support (Anthropic, OpenAI, and others for cloud; Ollama / Llamafile for local), tool/function
calling, and **structured extraction** of typed output. **User Context** is the first feature on this
path; **Ask AI** migrates onto it later. This replaces the prior direction of delegating to an
installed PI runtime over a Node shim ([ADR 0023](0023-ask-ai-delegates-auth-to-installed-pi.md)):
a shim makes the *user* supply a runtime (PI, or a system Node), which is not a shippable consumer
posture. A bundled JS runtime (e.g. embedding flue + Node) was also rejected — it ships and maintains
a whole second runtime and a Rust⟷Node IPC seam — when Mnema is already a Rust/Tauri app that can call
providers directly. rig-core keeps the agent loop, the broker/redaction, and the capture data all in
one Rust process, no IPC, nothing extra for the user to install.

The local-vs-cloud split is a per-user **Reasoning Engine** *choice* applied to both **Activity** and
**Conclusion** derivation, expressed as a rig-core provider/model selection: a **cloud** engine is an
HTTPS call to the provider with a **bring-your-own-key** credential stored in OS platform secret
storage (the **Capture Index Key Store** keychain boundary, never in `saveDirectory` or a config
file); a **local** engine points rig-core at a local endpoint (Ollama/Llamafile) and needs no key.
Mnema operates no backend and no token proxy — the user's key talks straight to their provider. Using
a cloud engine for continuous background derivation is its **own explicit opt-in**, separate from the
**Ask AI Setting**, with its own disclosure (continuous redacted egress + ongoing token cost) and
model selection. The privacy guarantee holds regardless of engine: a cloud engine sees only redacted
text passing through (never raw frame images or audio), and the assembled dossier is stored only
on-device — the cloud is stateless reasoning that never holds the whole profile.

**Considered Options**

We rejected the **user-supplied-Node shim** (the PI pattern): "first install Node/PI" is a non-starter
for a shipped product, and it is the dependency the move off PI was meant to escape. We rejected
**bundling Node + flue**: it keeps flue's agent harness but ships/maintains a second runtime plus an
IPC boundary on every AI call, and **User Context** derivation does not need an interactive agent
harness — it is batch structured generation ("summarize captures into Activities," "distill Activities
into Conclusions, return JSON"), which rig-core's structured extraction covers natively in Rust. We
rejected a **fixed local/cloud layer seam** (Activities local, Conclusions cloud): the real variable
is which machine the user has, so it is a per-user engine choice, not a layer boundary. We rejected
**Mnema-operated cloud access** (bundled key or token proxy): that needs Mnema servers and
Mnema-owned credentials, exactly what the all-native posture avoids.

**Consequences**

This reverses [ADR 0023](0023-ask-ai-delegates-auth-to-installed-pi.md) for new work — PI delegation
is abandoned in favor of Rust-native rig-core — and flue is dropped (it was only ever needed if the
agent loop lived in Node). 0023 continues to describe the *current* PI-based Ask AI until that feature
is migrated. Mnema now stores the user's provider key in the keychain (versus 0023's zero-credential
delegation), keeping 0023's spirit (no Mnema servers, no Mnema-owned credentials — still the user's
own key to their own provider). A **local** engine requires the user to run a local model server
(Ollama/Llamafile); a **cloud** engine requires only a pasted key and nothing installed. With the
cloud opt-in off and no local engine selected, **User Context** is unavailable — the same "no usable
runtime → feature unavailable" shape Ask AI already has. If **Ask AI**'s interactive tool-loop later
wants a heavier harness, rig-core's agent/tool modules cover it Rust-side without reintroducing Node.
