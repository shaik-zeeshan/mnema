// The `ui.main_surface` app setting (design frame 07 / PLAN slice 6): which of
// the two main surfaces — Timeline (`/`) or Overview (`/insights`) — the main
// window opens on. Out-of-box default is Timeline; the backend normalizes any
// missing/unknown value to "timeline".

import { invoke } from "@tauri-apps/api/core";

export type MainSurface = "timeline" | "overview";

/** Route for a surface. Overview is the insights route until slice 10 renames it. */
export function surfaceRoute(surface: MainSurface): string {
  return surface === "overview" ? "/insights" : "/";
}

export async function getMainSurfaceSetting(): Promise<MainSurface> {
  const value = await invoke<string>("get_main_surface_setting");
  return value === "overview" ? "overview" : "timeline";
}

export async function setMainSurfaceSetting(surface: MainSurface): Promise<void> {
  await invoke("set_main_surface_setting", { surface });
}
