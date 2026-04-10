# Turborepo + Bun monorepo

This repository uses a Bun workspace monorepo managed by Turborepo.

- `apps/desktop`: Tauri + SvelteKit desktop app
- `packages/*`: shared packages (when present)

## Workspace commands

Run these from the repository root:

- `bun run dev` — run desktop app development tasks (`turbo run dev --filter=desktop`)
- `bun run build` — build the desktop app (`turbo run build --filter=desktop`)
- `bun run check` — run checks for the desktop app (`turbo run check --filter=desktop`)
- `bun run tauri -- <args>` — run Tauri CLI via the root (for example: `bun run tauri -- dev`)
