# AGENTS

## Workspace
- This is a Bun workspace monorepo driven from the repo root with Turborepo. Root scripts are the source of truth.
- Current app layout is minimal: `apps/desktop` is the only app, and there are no existing `packages/*` entries yet.

## Commands
- Run from the repo root unless a note says otherwise.
- `bun run dev` runs `turbo run dev --filter=desktop`.
- `bun run build` runs the desktop frontend build through Turbo.
- `bun run check` is the main frontend verification step and currently runs `svelte-kit sync && svelte-check --tsconfig ./tsconfig.json` for `apps/desktop`.
- `bun run tauri -- dev` is the correct way to launch the desktop app in Tauri dev mode from the root.
- For Rust-only verification, use `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.

## Desktop App Boundaries
- Frontend code lives in `apps/desktop/src`.
- Native Tauri/Rust code lives in `apps/desktop/src-tauri/src`.
- Tauri command registration happens in `apps/desktop/src-tauri/src/lib.rs`; wire new commands there.
- The current Svelte entry UI is `apps/desktop/src/routes/+page.svelte`.

## Framework Quirks
- The desktop frontend is a SvelteKit SPA, not SSR: `apps/desktop/src/routes/+layout.ts` sets `ssr = false`, and `apps/desktop/svelte.config.js` uses `@sveltejs/adapter-static` with `fallback: "index.html"`.
- Vite is pinned to port `1420` with HMR on `1421` in Tauri mode. Do not casually change those ports; `tauri.conf.json` expects `http://localhost:1420`.
- `apps/desktop/src-tauri/tauri.conf.json` runs `beforeDevCommand: bun run dev` and `beforeBuildCommand: cargo clean --manifest-path src-tauri/Cargo.toml && bun run build`. Expect Tauri builds to clean the Rust crate first.

## Change Strategy
- If a feature is explicitly desktop-app-specific, implement it in `apps/desktop` / `apps/desktop/src-tauri` as needed.
- Otherwise, prefer creating a separate Rust crate for the intended task and calling it from the desktop Tauri crate instead of putting general-purpose logic directly into `apps/desktop/src-tauri`.
- This repo does not currently define a Cargo workspace, so an added crate will also need to be wired into `apps/desktop/src-tauri/Cargo.toml` as a dependency.

## Verification
- For frontend/UI changes, run `bun run check`.
- For Rust/Tauri changes, run `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`.
- For cross-stack changes or anything affecting packaging/build wiring, run both.
