---
status: accepted
---

# Quick Recall and the Insights Chat share one engine and one persistent conversation store

The **Insights** workspace adds a **Chat** sub-surface: a persistent, searchable conversation
workspace that answers questions over the user's history and renders graphical answers inline.
Rather than building it as a second assistant, **Chat** and the existing **Quick Recall** **Ask AI**
share **one engine and one conversation store** — they are two doors into the same conversations.
**Quick Recall** is the fast, summon-anywhere door (ask, glance, go); **Chat** is the sit-down deep
door; a thread started in one resumes in the other ("if the user wants to go deep, they open it in
Chat"). This means conversations now **persist to disk**, which **reverses the deliberate
disk-ephemerality of [ADR 0027](0027-ask-ai-threads-complete-in-background-and-resurrect-from-transcript.md)**.

Persisted conversations are sensitive — a saved chat about the user's life is as revealing as the
**User Context** dossier it discusses — so they are stored inside the **Encrypted Capture Index**,
subject to **Retention Policy**, and cleared by **Wipe User Context**, the same protections the
dossier gets. Both doors run the same Rust-side `rig-core` engine
([ADR 0028](0028-ai-features-call-models-rust-side-via-rig-core.md)) and reach capture data only
through the same **Brokered Capture Access** tools (`search`/`timeline`/`show_text`/`recall_context`),
so persistence changes where conversations are *stored*, not what the model is *allowed to see*.

**Considered Options**

We rejected **two separate assistants** (an ephemeral Quick Recall plus an independent persistent
Chat): they would diverge, duplicate the engine and tool wiring, and force the user to choose the
"right" one up front instead of escalating a quick question into a deep session in place. We rejected
**keeping Quick Recall ephemeral and persisting only Chat**: with one shared store, the natural
"open this Quick Recall thread in Chat" handoff requires the Quick Recall thread to already be
persisted, so a split-persistence model would defeat the unification. We rejected **plaintext or
unprotected conversation storage**: conversation content is at least as sensitive as the encrypted
dossier and must share its encryption, retention, and wipe story.

**Consequences**

ADR 0027's background-completion / resurrect-from-transcript machinery (built for an *ephemeral*
thread) is superseded by genuine persistence: a thread is simply saved and reopened, not
reconstructed from a transcript. The privacy posture shifts — conversations about captured life now
live on disk by design — so this persistence is disclosed and governed by the same **Wipe** and
retention controls as the dossier, and **Quick Recall**'s prior "persists nothing to disk" promise no
longer holds and must be updated wherever it is stated. Search-over-chats and a chat-history list
(the workspace shell) become first-class, since conversations are now durable objects.
