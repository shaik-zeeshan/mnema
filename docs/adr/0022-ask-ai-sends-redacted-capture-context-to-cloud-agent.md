---
status: accepted
---

# Ask AI sends redacted capture context to a third-party cloud agent

Mnema's **Quick Recall** overlay adds an **Ask AI** action that answers questions over retained capture by running a PI Agent SDK conversation in the cloud. This is Mnema's first outbound flow of captured-derived content to a third-party service other than the user-driven `mnema` CLI, so it is gated by an explicit, off-by-default, standing **Ask AI Setting** with a disclosure that questions send redacted capture context to PI's cloud, rather than shipping enabled.

The context handed to PI rides the same **Brokered Capture Access** redaction and retention policy that backs the CLI: secrets are redacted and deleted/tombstoned data is excluded before anything leaves the machine. **Ask AI** reuses that broker policy/query code rather than reaching app-infra rows or media directly, and the in-app agent is recorded in access audit history. The consent gate is a durable setting (toggling it off is the revocation), not a time-bounded **CLI Access Grant**.

**Considered Options**

We rejected reusing the broker's time-bounded grant model (1h/24h/7d) as the consent gate: grant expiry exists to bound *untrusted, intermittent* external tools, and re-prompting the user for their own deliberately-enabled in-app assistant on a credential-rotation cadence is friction with no added safety, since the setting is one toggle to revoke. We also rejected giving the in-app agent privileged raw app-infra access on the theory that "it is the user's own data," because the context still leaves the machine to a cloud model, which is exactly the boundary secret redaction exists to defend.

**Consequences**

The data the **Ask AI** agent sees is deliberately less than the data the user sees in **Quick Search**: the user reads full-fidelity local results while the cloud agent receives only redacted broker context for the same query. Enabling **Ask AI** is meaningful consent and must be surfaced with disclosure in onboarding and **Access Settings**; **Ask AI** is unavailable until Mnema onboarding is complete, inherited from the **Brokered Capture Access** precondition.
