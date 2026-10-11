import { formatBytes } from "$lib/settings/state/format";

/** The `mx-progress__meta` line for a running model download. */
export interface DownloadMeta {
  title: string;
  detail: string;
  /** null = unknown total → indeterminate bar. */
  percent: number | null;
}

export function downloadMeta(
  name: string,
  progress: { status: string; downloadedBytes: number; totalBytes: number | null } | null,
): DownloadMeta {
  const title =
    progress?.status === "installing"
      ? `Installing ${name}`
      : progress?.status === "starting" || !progress
        ? "Starting download"
        : `Downloading ${name}`;
  const done = progress?.downloadedBytes ?? 0;
  const total = progress?.totalBytes ?? 0;
  if (total <= 0) {
    return { title, detail: done > 0 ? formatBytes(done) : "", percent: null };
  }
  const percent = Math.min(100, Math.round((done / total) * 100));
  return { title, detail: `${formatBytes(done)} of ${formatBytes(total)} · ${percent}%`, percent };
}
