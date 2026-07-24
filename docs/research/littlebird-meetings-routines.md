# Littlebird "Meetings" and "Routines" vs Mnema Triggers

Research date: 2026-07-23. Primary sources: littlebird.ai marketing pages, feature pages, changelog, launch blog post, and launch press. All Littlebird claims cite the source URL; anything not confirmed on a primary source is marked **UNVERIFIED**.

**Product identification note**: the prompt named "littlebird.app" — that domain does not resolve (DNS ENOTFOUND). The actual product is **Littlebird at `littlebird.ai`**, the "full-context AI assistant" for Mac (also Windows/iOS/Android/browser), founded by Alexander Green, Alap Shah, and Naman Shah, launched publicly 2026-03-24 with an $11M seed led by Lotus Studio ([littlebird.ai blog](https://littlebird.ai/blog/littlebird-raises-11m-to-build-the-first-full-context-ai), [PR Newswire](https://www.prnewswire.com/news-releases/littlebird-raises-11-million-to-launch-the-only-ai-that-already-knows-what-youre-working-on-302721664.html), [TechCrunch](https://techcrunch.com/2026/03/23/littlebird-raises-11m-to-capture-context-from-your-computer-so-you-can-query-your-data/)).

## What Littlebird is

- Ambient screen-reading + meeting-transcribing AI assistant: "reads the structured content of every application on your screen and transcribes your meetings in real time" ([PR Newswire](https://www.prnewswire.com/news-releases/littlebird-raises-11-million-to-launch-the-only-ai-that-already-knows-what-youre-working-on-302721664.html)).
- Reads text of the active window (not screenshots/video); skips minimized apps, incognito windows, password/credit-card fields ([TechCrunch](https://techcrunch.com/2026/03/23/littlebird-raises-11m-to-capture-context-from-your-computer-so-you-can-query-your-data/); privacy exclusion controls in [changelog Dec 2025](https://littlebird.ai/changelog/december-2025)).
- Cloud product: data encrypted on AWS servers, SOC 2 / GDPR / CCPA ([littlebird.ai](https://littlebird.ai/)). This is the architectural opposite of Mnema's local-first encrypted store.
- Four core surfaces: Chat, Meeting Notes, Routines, and "Hummingbird" (an overlay/keyboard-shortcut assistant usable mid-meeting) ([littlebird.ai](https://littlebird.ai/)).

## How Meetings ("Meeting Notes") works

- **Detection — calendar first, app-detection second.** Users connect a calendar to see upcoming meetings; "Join your meeting (or launch from Littlebird) and transcription starts automatically" ([/features/meeting-notes](https://littlebird.ai/features/meeting-notes)). Apple Calendar events feed a "Coming up" section ([changelog Mar 2026](https://littlebird.ai/changelog/march-2026)). As of April 2026 it also "begin[s] recording automatically when it detects a meeting app like Google Meet or Zoom," with an advance notice, plus detection of a non-working mic at meeting start ([changelog Apr 2026](https://littlebird.ai/changelog/april-2026)).
- **Botless, local audio capture**: "Littlebird transcribes from your computer's audio — not by joining as a bot… completely invisible to other meeting participants and works with any meeting app" (Zoom, Teams, Meet, anything) ([/features/meeting-notes](https://littlebird.ai/features/meeting-notes)).
- **Output**: live transcript during the call; "your summary, action items, and transcript after the call"; April 2026 pitch: "From live transcript to shared actions in seconds" ([/features/meeting-notes](https://littlebird.ai/features/meeting-notes), [changelog index](https://littlebird.ai/changelog)). Notes are editable: refine summaries, add action items manually, pause or delete a note ([/features/meeting-notes](https://littlebird.ai/features/meeting-notes)).
- **Pre-meeting prep** (no Mnema equivalent): open an upcoming meeting, click "Prep for meeting" — "Littlebird pulls together what you need to know, including context about attendees and past meetings" and emails/projects ([changelog Mar 2026](https://littlebird.ai/changelog/march-2026), [/features/meeting-notes](https://littlebird.ai/features/meeting-notes)).
- **In-meeting**: Hummingbird overlay for querying Littlebird during a call via shortcut ([/features/meeting-notes](https://littlebird.ai/features/meeting-notes)).
- **Languages**: user picks transcription language in Settings > Meetings incl. a multilingual option ([changelog Apr 2026](https://littlebird.ai/changelog/april-2026)); "auto-detect language in meeting notes" is a Pro-tier perk ([/pricing](https://littlebird.ai/pricing)). 10+ languages on the free tier (**UNVERIFIED** — seen only in third-party summaries of the pricing page).
- **Rollout**: announced "coming soon" Dec 2025 (Plus early access), GA to all users Jan 2026 ([changelog Dec 2025](https://littlebird.ai/changelog/december-2025), [changelog index](https://littlebird.ai/changelog)).

## How Routines works

- **What it is**: scheduled AI prompts — "Routines run on your schedule, proactively delivering the information you need" ([/features/routines](https://littlebird.ai/features/routines)).
- **Configuration**: "write a prompt describing what you want (just like chatting with Littlebird), give it a name, and choose how often you want it to run." "No scripts, no logic trees, no technical setup." Edit prompt/frequency or delete anytime ([/features/routines](https://littlebird.ai/features/routines)).
- **Trigger model: schedule only.** Recurring daily/weekly/monthly cadences (examples: daily briefing, weekly competitor monitoring, monthly project analysis). There is **no event-based trigger** (no "when a meeting ends" / "when app X opens") anywhere on the feature page or changelog ([/features/routines](https://littlebird.ai/features/routines), [changelog](https://littlebird.ai/changelog)).
- **Templates**: ready-made templates or from scratch; a two-column create UI for picking templates and editing instructions; a redesigned list with search/sort/grid (Apr 2026) ([/features/routines](https://littlebird.ai/features/routines), [changelog Dec 2025](https://littlebird.ai/changelog/december-2025), [changelog Apr 2026](https://littlebird.ai/changelog/april-2026)).
- **Output**: each update is a private delivery with "a chat icon that lets you open a conversation with Littlebird about that specific update" — i.e., run result → follow-up chat, same shape as Mnema's runs-are-conversations ([/features/routines](https://littlebird.ai/features/routines)). Exact delivery surface (push notification vs in-app feed vs email) is **UNVERIFIED** — primary pages say only "proactively delivering"; Routines are also readable on the mobile companion apps ([changelog Dec 2025](https://littlebird.ai/changelog/december-2025)).
- **Tooling inside runs**: "Integrations now work in Routines too" (Apr 2026) — routine runs can reach connected external services; MCP access exists at the Power tier ([changelog Apr 2026](https://littlebird.ai/changelog/april-2026), [/pricing](https://littlebird.ai/pricing)). Opposite of Mnema's sealed toolbox.
- **Gating**: Basic (free) = daily usage credits for chats and routines + limited meeting notes; Plus $17/mo annual ($20 monthly) = unlimited meeting notes, daily+monthly credits; Power $42/mo = 2.5× credits + MCP; Pro from $100/mo = 5–12× credits + language auto-detect; Team from $17/seat; Enterprise custom ([/pricing](https://littlebird.ai/pricing)).

## Feature-by-feature comparison vs Mnema Triggers

Mnema side from [`docs/triggers/CONTEXT.md`](../triggers/CONTEXT.md), ADR 0057/0058.

| Dimension | Littlebird | Mnema Triggers |
|---|---|---|
| Trigger vocabulary | Two separate features: Meetings (implicit, always-on once set up) and Routines (schedule-only) | One feature: Condition × Prompt. v1 conditions: Meeting Ends, App Opened, Schedule |
| Event conditions | None — Routines are time-based only; meeting capture is its own feature, not user-composable | Meeting Ends and App Opened are first-class, composable with any prompt |
| Meeting detection | Calendar integration + (Apr 2026) meeting-app detection with an advance notice; user confirms/starts transcription | Mechanical: Core Audio per-process mic-hold by allowlisted conferencing app or browser-with-meeting-URL, ≥5 min, 2 min release grace. No calendar, no user action |
| Meeting output | Live transcript + post-call summary, action items, transcript; editable notes; shared actions | Post-meeting Trigger Run: user-authored prompt over the capture window, rendered as a document-view conversation |
| Pre-meeting | "Prep for meeting" pulls attendee/email/past-meeting context before the call | None — Triggers only fire after conditions occur |
| In-meeting | Hummingbird overlay Q&A during the call | None (Quick Recall exists but isn't meeting-aware) |
| Prompt model | Free-text NL prompt + name + frequency; template gallery | Free-text Prompt + starter templates; wizard (Condition → Prompt → Review); share/import as canonical JSON |
| Context assembly | Ambient screen+meeting memory, learned preferences | Explicit: firing context, User-Context conclusions, speaker identity, previous runs of the same trigger |
| Run output surface | Private update with a chat icon → follow-up conversation | Run *is* a conversation (origin=trigger), document view, follow-ups as chat beneath |
| Tool access during runs | Integrations + web + (Power) MCP available inside Routines | Sealed toolbox: read-only inward tools only, no web/MCP — deliberate anti-exfiltration invariant |
| Delivery | In-app/mobile-synced updates; notification specifics UNVERIFIED | macOS notification → opens the run conversation; skipped runs never notify |
| Flap protection | Not documented | Cooldown (10 min), App Opened 30-min away-gap, meeting release grace |
| Data-readiness | Live cloud transcription, so results immediate | Readiness Wait (≤15 min) for local transcription/diarization to finish |
| Mobile | iOS/Android companions, routines synced | None (desktop only) |
| Architecture | Cloud (AWS), SOC 2 | Local-first, encrypted SQLCipher DB, offline-capable |
| Pricing gate | Credits per tier; unlimited meeting notes at $17/mo+ | No per-run metering; Provider Gate only (needs a configured AI provider) |

## Gaps / ideas relevant to Mnema

What Littlebird has that Triggers don't:

1. **Pre-meeting prep** — a *before*-the-event surface (attendees, past meetings, related docs) driven by calendar. Mnema's conditions are all reactive; a "Meeting Starts / Upcoming Meeting" condition (calendar or the same mic-hold onset) would be the natural extension, and Mnema's past-runs + recall_context assembly already fits the "context from past meetings" half.
2. **Routine template gallery** — a browsable, searchable library of ready-made prompt+schedule combos (daily briefing, weekly summary). Mnema has starter templates and Share/Import JSON; a curated in-app gallery is the low-cost version of this.
3. **Follow-the-user delivery** — mobile companions and cross-device sync mean routine output reaches you off-desktop. Mnema delivery is a macOS notification only; nearest local-first analog would be an outward Delivery option (e.g. Slack/email), which CONTEXT.md already reserves as a future Delivery — not an open tool.
4. Smaller ones: live transcript visibility during the meeting; editable/refinable run output; mic-not-working detection at meeting start (Mnema's Skipped Run covers the aftermath but doesn't warn upfront); integrations available inside scheduled runs (Mnema rejects this deliberately — sealed toolbox).

What Mnema Triggers have that Littlebird doesn't:

1. **Event-based conditions** — Meeting Ends and App Opened composed with arbitrary prompts. Littlebird routines can't react to anything; its meeting feature isn't user-programmable.
2. **Calendar-free, zero-action meeting detection** — mic-hold detection fires with no calendar connection and no "start transcribing" click, and absorbs drop/rejoin via the grace period.
3. **Sealed toolbox** — structural prompt-exfiltration safety for unattended runs, which matters more for Mnema (shareable trigger JSON) and is genuinely absent in Littlebird's integrations-in-routines model.
4. Local-first/no-cloud, no usage credits, share/import of trigger definitions, skipped-run semantics ("notifications are only ever good news"), cooldown/flap protection, readiness wait for pipeline completeness.

## Sources

Primary (littlebird.ai):
- https://littlebird.ai/ — product overview, feature nav, privacy/compliance, platforms
- https://littlebird.ai/features/routines — Routines mechanics, configuration, templates, chat-per-update
- https://littlebird.ai/features/meeting-notes — botless local-audio capture, calendar flow, prep, editing, Hummingbird
- https://littlebird.ai/pricing — tiers, credits, meeting-note limits, MCP/auto-language gating
- https://littlebird.ai/changelog — month index; Meeting Notes GA (Jan 2026), language auto-detect (Feb 2026)
- https://littlebird.ai/changelog/december-2025 — Meeting Notes early access, Routines on mobile, two-column create UI, privacy exclusions
- https://littlebird.ai/changelog/march-2026 — Prep for meeting, Apple Calendar "Coming up", guided onboarding
- https://littlebird.ai/changelog/april-2026 — auto-record on meeting-app detection, mic check, language settings, Routines redesign, integrations in Routines
- https://littlebird.ai/blog/littlebird-raises-11m-to-build-the-first-full-context-ai — launch post, founders' framing

Launch press (founder statements):
- https://www.prnewswire.com/news-releases/littlebird-raises-11-million-to-launch-the-only-ai-that-already-knows-what-youre-working-on-302721664.html — company announcement, founders, investors, product description
- https://techcrunch.com/2026/03/23/littlebird-raises-11m-to-capture-context-from-your-computer-so-you-can-query-your-data/ — active-window text reading, exclusions, founder interview

Not usable: `littlebird.app` (DNS does not resolve; wrong domain in the research request).
