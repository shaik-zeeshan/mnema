// Ambient type declarations.
//
// Makes `~icons/<collection>/<name>` imports resolve to Svelte components for
// unplugin-icons (see vite.config.js). Svelte 5 note: the generated components
// compile in legacy mode, which is safe here because the app does not force
// `compilerOptions.runes: true` in svelte.config.js.
/// <reference types="unplugin-icons/types/svelte" />

declare global {
  // SvelteKit's App namespace — left empty; extend as needed.
  // eslint-disable-next-line @typescript-eslint/no-namespace
  namespace App {}
}

export {};
