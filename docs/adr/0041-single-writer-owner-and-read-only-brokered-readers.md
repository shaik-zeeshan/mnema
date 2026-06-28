# Single Capture Index Owner; brokered consumers are genuinely read-only

## Status

Accepted.

## Context

The Encrypted Capture Index (one SQLCipher/WAL `app.sqlite3`) was producing a flood of
`SQLITE_BUSY` "database is locked" errors. Investigation found three compounding causes,
not the one we first assumed:

1. **Brokered consumers were writing to the live database.** `AppInfra::initialize_read_only`
   — used by the `mnema` CLI (a separate OS process, once per invocation) and by the in-app
   Ask AI agent (a *second* pool inside the desktop process, once per turn) — shared the
   desktop app's open path (`db::Database::initialize`). "Read-only" only meant "skip the
   startup maintenance workers." Every brokered open therefore ran `MIGRATOR.run`
   (creates/locks `_sqlx_migrations`) and `PRAGMA journal_mode=WAL` (a lock-taking write)
   against the database the desktop app was actively writing.

2. **In-process writer-writer lock-upgrade deadlock.** The desktop pool was
   `max_connections(4)` with sqlx's default deferred (`BEGIN`) transactions. In WAL, when two
   connections each take a read lock and then try to upgrade to a write lock, SQLite returns
   `SQLITE_BUSY` *immediately* — `busy_timeout` does not apply to an upgrade conflict because
   waiting cannot resolve it. This produced "locked" errors that ignored the 5 s timeout.

3. **`synchronous=FULL` (the unset default)** lengthened how long each commit held the write
   lock for no benefit under WAL.

Constraints that shaped the fix: `mnema` data commands run against an on-disk grant and must
work with the desktop app **closed** (including closed-after-crash, where the `-wal` is dirty);
and the brokered read path must never block or corrupt the live capture writers.

## Decision

**Exactly one Capture Index Owner.** The desktop app is the sole process that writes the
Encrypted Capture Index and the sole process that runs migrations. Every other opener is a
Brokered Reader.

- **Brokered Reader open mode.** The `mnema` CLI and Ask AI open a read-write OS handle made
  logically read-only with `PRAGMA query_only=ON`; they **never** run the migrator and
  **never** set `journal_mode`. Because they only take read locks, in WAL they never collide
  with the Owner. A read-write OS handle (rather than strict `SQLITE_OPEN_READONLY`) is
  required so a brokered read can still open and recover the WAL sidecars after an Owner crash.

- **Writer Connection + Reader Pool inside the Owner.** The Owner writes through a single
  `max_connections(1)` Writer Connection and reads through a separate multi-connection Reader
  Pool. One writer connection means there is never more than one write transaction in flight,
  so the upgrade deadlock is *structurally* impossible, while WAL keeps Owner reads concurrent
  with the Owner write.

- **Pragma tuning.** `synchronous=NORMAL` on the Writer Connection (WAL-safe; only at-risk
  window is the last transaction on an OS/power crash). Keep WAL and `busy_timeout` on every
  handle (Writer Connection ~10 s). Set `journal_size_limit` to bound `-wal` growth when
  long-lived readers delay checkpoints.

- **No application-level retry.** `busy_timeout` is the single backstop. A *persistent* lock
  after these changes signals a real defect (e.g. a write path that bypassed the Writer
  Connection) and should surface rather than be silently retried.

## Considered options

- **Just raise `busy_timeout` / add a retry-with-backoff.** Rejected as the primary fix: it
  cannot resolve the lock-upgrade deadlock (which fails instantly regardless of timeout) and
  masks the brokered-writes-to-live-DB bug instead of removing it.
- **Force `BEGIN IMMEDIATE` on every write transaction (keep the 4-connection pool).** Smallest
  diff and it makes `busy_timeout` apply, but it is fragile: sqlx's `begin()` is deferred, so
  every transaction start must be converted by hand and any missed write path silently
  re-introduces the deadlock. The single Writer Connection makes the same guarantee
  structurally, so `BEGIN IMMEDIATE` is unnecessary.
- **Single `max_connections(1)` pool for everything.** Eliminates the deadlock but serializes
  reads behind writes inside the Owner, discarding WAL's concurrency and risking UI jank /
  pipeline backpressure. The Writer/Reader split keeps the concurrency.
- **Strict `SQLITE_OPEN_READONLY` for brokered readers (plus a crash fallback).** A harder
  OS-level guarantee, but a strict read-only handle cannot open a WAL database with a dirty
  `-wal` and no live owner, breaking `mnema search` after a crash unless a read-write fallback
  is added anyway. `query_only` reaches the same logical guarantee in one path.

## Future decisions (deferred levers, intentionally not done now)

- **Targeted retry/backoff** can be added later if residual transient `SQLITE_BUSY` is actually
  observed in practice — as a narrow fix at the contended call site, not a blanket wrapper.
- **Periodic `PRAGMA wal_checkpoint(TRUNCATE)` on idle** if `journal_size_limit` + the default
  `wal_autocheckpoint` prove insufficient to bound `-wal` growth under many concurrent readers.
- **Ask AI reusing the Owner's Reader Pool directly** (instead of constructing its own Brokered
  Reader) is a possible in-process simplification, deferred to keep the brokered path uniform
  across the CLI and Ask AI for now.
- **Perf pragmas** (`mmap_size`, `cache_size`, `temp_store=MEMORY`) are out of scope here; they
  are throughput levers, not locking fixes.

See `crates/app-infra/CONTEXT.md` for the resolved terms: **Capture Index Owner**,
**Writer Connection**, **Reader Pool**, **Brokered Reader**.
