# A build newer than the owner's update window shows a permanent banner and offers rollback only when safe

## Status

Accepted (2026-11-01); not yet implemented. Prompted by the Linux port; applies on every platform.
Specifies the open resolution in [`docs/licensing/CONTEXT.md`](../licensing/CONTEXT.md), "Documented
edge case — fresh install after Update Window lapse" (banner, no lock), and adds rollback.
Adds one Receipt Refresh trigger to [ADR 0055](0055-receipt-refresh-is-event-driven-from-unhealthy-states.md).
Related: [ADR 0053](0053-licenses-activate-once-per-machine-via-a-signed-activation-receipt.md),
[ADR 0054](0054-licensing-moves-onto-licensegate.md). Source: `docs/linux-port-report.md` §4.12,
§10 decision 9.

## Context

A licence grants permanent Capture plus updates released within its Update Window. Only the
built-in updater enforces the window: it declines builds dated after the owner's
`update_through_ms` (`apps/desktop/src-tauri/src/app_updates.rs:411-425`, `build_out_of_window`;
the update is marked not installable at `:469-480`). Nothing locks: a lapsed owner keeps their
covered build (CONTEXT.md, Perpetual Fallback).

Two channels bypass the updater:

- **Linux package managers.** The AUR (and any future apt repo or Flathub) serves the newest build
  to everyone, and the in-app updater is off there.
- **Manual downloads.** On every platform the website serves the newest build.

The app already notices when the *running* build is past the window: `running_build_window_gate`
(`app_updates.rs:427-445`) compares the compile-time `MNEMA_BUILD_DATE_MS` with the window. But it
reuses `AvailableOutOfWindow`, the same state as "a newer update exists but is past your window",
and shows only an updates status line, "Update window has lapsed"
(`apps/desktop/src/lib/update-window.ts:61-62`). Most owners will never see it.

How others handle it: none of them restricts the channel; the newer build checks itself.

- **Sublime Text** ships its own apt/yum/pacman repos. An uncovered build shows "LICENSE UPGRADE
  REQUIRED" in the title bar and keeps working. (Source: a staff reply on Sublime's forum, not
  official docs.)
- **JetBrains** gives a fallback licence for an older version; newer versions refuse to activate.
  Since PyCharm 2025.1 and IntelliJ IDEA 2025.2 they fall back to a free core feature set instead.
- **Sketch** keeps the last eligible version working; newer ones say the licence isn't eligible.

Rolling back has a trap. The database migrator runs with sqlx's default `ignore_missing = false`
(`crates/app-infra/src/db.rs:20`, run at `:136`). sqlx 0.8.6 then returns `VersionMissing` for any
applied migration the binary doesn't know (`sqlx-core-0.8.6/src/migrate/migrator.rs:40`). So an
older build opening a database that a newer build migrated refuses to start, and startup has no
recovery path. There are 61 migrations so far, so many releases add one.

## Decision

**1. No lock.** A build dated after the owner's window keeps full Capture on every platform.

**2. A distinct "this version isn't covered" state, shown as a permanent banner.** It is split from
"a newer update is past your window". The banner sits app-wide, can be collapsed for the current
launch, and returns on every launch: "This version isn't covered by your licence. Your updates
ended on ⟨date⟩." Buttons: **Renew** and, when safe, **Roll back to ⟨covered version⟩**.

**3. Refresh before showing it.** Entering this state fires one Receipt Refresh first, so an owner
who has just renewed never sees the banner. This is an event from an unhealthy state, in line with
ADR 0055.

**4. The rollback target** is the newest build released within the owner's window. Covered builds
stay downloadable forever (already required by CONTEXT.md).

**5. How rollback runs:**

- **macOS, .deb, .rpm:** the built-in updater, pointed at the covered release with a version
  comparator that allows a downgrade (`tauri-plugin-updater` 2.10.1,
  `UpdaterBuilder::version_comparator`, `src/updater.rs:184`).
- **Arch** (updater off): show the one command, `sudo pacman -U <covered package URL>`, and how to
  stop the next system update from upgrading it again (`IgnorePkg`).

**6. Rollback is offered only when safe.** Each release manifest records the highest database
migration version it contains. The app compares that with the newest migration applied to its
database. If the database is ahead, the Roll back button is hidden and the banner says: "Rolling
back isn't possible because this version updated your data. Renew to keep it, or keep using it as
is."

## Considered options

- **Keep the quiet status line.** Honour-system with a notice almost nobody reads.
- **Pause recording in uncovered builds (the JetBrains model).** Rejected: a normal system update
  would move lapsed Arch owners onto a build that won't record, it reverses CONTEXT.md's "does not
  hard-lock", and a renewal that hasn't reached the machine yet would block a paying owner.
- **Banner first, then pause after 14 days.** Rejected for the same reasons, delayed.
- **AppImage instead of the AUR,** so the updater gate applies on Arch. Rejected: worse fit for Arch
  users, and it still doesn't cover a manual download.
- **Make every future migration readable by older builds** (`ignore_missing` plus an additive-only
  rule). Rejected: a permanent constraint on all schema work for a convenience feature.

## Consequences

- A lapsed owner who ignores the banner can keep updating through a package manager or the
  website. Accepted: the source is AGPL, so a determined user can build any version anyway.
- Release automation must publish, for every version and forever: the installers, an Arch package
  for `pacman -U`, and a per-version updater manifest carrying the migration high-water mark.
- Rollback will often be unavailable, because migrations are frequent. The banner says why instead
  of offering a button that would leave the app unable to start.
- CONTEXT.md's edge-case section should point here, and its wording "directs the owner to the newest
  build their License covers" now means this banner.
