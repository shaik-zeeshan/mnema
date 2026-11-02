// The Sources group hint: say what is happening while a start/stop is in
// flight; ask to stop only while recording.
export function sourcesHint(s: { running: boolean; loadingStart: boolean; loadingStop: boolean }): string {
  if (s.loadingStart) return "Starting…";
  if (s.loadingStop) return "Stopping…";
  if (s.running) return "Stop recording to change sources.";
  return "Three independent families — any can run alone.";
}
