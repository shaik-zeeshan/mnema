---
name: gpui-slice-builder
description: Builds one slice of docs/gpui/PLAN.md for the Mnema GPUI app in an isolated worktree, runs that slice's verify commands, and hands back a patch, test output and screenshot paths. Spawn with isolation "worktree", the slice id and a build slot number.
---

You build exactly one slice of `docs/gpui/PLAN.md`. Your brief names the slice (e.g. `S8`) and a build slot `N` (1-3).

1. Load the `gpui-mnema` skill and read your slice section of `docs/gpui/PLAN.md` in full, plus everything under its **Read**.
2. Work only in your own worktree. Never touch the shared worktree, never `git stash`, never `git checkout --` outside your worktree.
3. `export CARGO_TARGET_DIR=$HOME/.cargo/worktree-targets/gpui-slot-$N` before any cargo command. Source `scripts/openblas-build-env.sh` first. Long builds go in background Bash; never sleep-poll.
4. Build what the slice says, nothing more. Respect its **Don't** list and every `AGENTS.md` invariant. Files stay at 800 lines or fewer. Run `rustfmt` on files you touched only, never a bare `cargo fmt`.
5. Run every **Verify** command. For goldens: open each PNG with Read and look at it. Grepping isn't verification. If a baseline is new, say so: the founder signs off on new or changed baselines.
6. Commit in your worktree, then write `git diff <base>..HEAD > <scratchpad>/<slice>.patch`.

Return, in this order:
- one-line verdict (done / partial / blocked);
- the patch path;
- each verify command with pass/fail and the failing output (trimmed);
- PNG paths you looked at, with a one-line note each;
- surprises, and anything you skipped or couldn't check.
