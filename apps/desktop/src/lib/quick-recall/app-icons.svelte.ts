// Real app icons for Quick Recall surfaces (result cards, detail pane, filter
// app value list). A tiny reactive cache over the existing `resolve_app_icons`
// command, which accepts bundle ids OR display names (it falls back to a
// display-name catalog — the Ask-AI timeline chips rely on the same behavior).
// Each surface ensures+looks up with ONE identifier convention: frames use
// `appBundleId ?? appName`, the filter value list uses the row's app name.
// Unresolvable identifiers stay absent and the surface keeps its letter/text
// fallback. Module-level singleton on purpose: icons are process-stable, and
// the Quick Recall webview is hidden, not destroyed (same lifetime story as
// the search store singleton).
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { AppIconResolution } from "$lib/app-privacy-exclusion";

class AppIconCache {
  // identifier -> asset-protocol src, reactive so <img> appears on resolve.
  #srcs = $state<Record<string, string>>({});
  // Everything ever sent to the backend (hit or miss) so each identifier is
  // resolved at most once per app run; cleared per-identifier on invoke error
  // so a transient failure retries on the next ensure.
  #requested = new Set<string>();

  src(identifier: string | null | undefined): string | null {
    const id = identifier?.trim();
    return id ? (this.#srcs[id] ?? null) : null;
  }

  ensure(identifiers: Array<string | null | undefined>): void {
    const missing: string[] = [];
    for (const raw of identifiers) {
      const id = raw?.trim();
      if (id && !this.#requested.has(id)) {
        this.#requested.add(id);
        missing.push(id);
      }
    }
    if (missing.length === 0) {
      return;
    }
    void invoke<AppIconResolution[]>("resolve_app_icons", {
      request: { bundleIds: missing },
    })
      .then((resolutions) => {
        const next = { ...this.#srcs };
        for (const icon of resolutions) {
          if (icon.iconPath) {
            next[icon.bundleId] = convertFileSrc(icon.iconPath);
          }
        }
        this.#srcs = next;
      })
      .catch(() => {
        for (const id of missing) {
          this.#requested.delete(id);
        }
      });
  }
}

export const appIcons = new AppIconCache();
