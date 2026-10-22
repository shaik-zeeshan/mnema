import type { StorageUsage } from "$lib/types/app-infra";

export interface MeterPart {
  label: string;
  bytes: number;
  /** CSS colour token for the bar segment + legend swatch. */
  color: string;
  /** Share of the total, 0–100. */
  pct: number;
}

export function storageTotal(u: StorageUsage): number {
  return u.screenBytes + u.microphoneBytes + u.systemAudioBytes + u.indexBytes + u.modelsBytes;
}

/** The meter's five parts in mockup order (screen · mic · system · index · models). */
export function meterParts(u: StorageUsage): MeterPart[] {
  const total = storageTotal(u);
  return [
    { label: "screen", bytes: u.screenBytes, color: "var(--app-source-screen)" },
    { label: "mic", bytes: u.microphoneBytes, color: "var(--app-source-mic)" },
    { label: "system", bytes: u.systemAudioBytes, color: "var(--app-source-sysaudio)" },
    { label: "index", bytes: u.indexBytes, color: "var(--chart-grey-4)" },
    { label: "models", bytes: u.modelsBytes, color: "var(--chart-grey-2)" },
  ].map((p) => ({ ...p, pct: total > 0 ? (p.bytes / total) * 100 : 0 }));
}

/** Average capture (screen + mic + system) per recorded day; null before any day is on disk. */
export function perDayBytes(u: StorageUsage): number | null {
  if (u.captureDays <= 0) return null;
  return (u.screenBytes + u.microphoneBytes + u.systemAudioBytes) / u.captureDays;
}
