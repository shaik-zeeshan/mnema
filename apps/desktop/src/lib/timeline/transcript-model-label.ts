// The audio drawer header's model label: a short lowercase name in the strip
// ("parakeet v3", "deepgram nova-3"), the full provider · model id in its tooltip.

export interface TranscriptModelLabel {
  label: string;
  tip: string;
}

const PROVIDER_NAME: Record<string, string> = {
  local_whisper: "Local Whisper",
  apple_speech_on_device: "Apple Speech (on-device)",
  parakeet: "Parakeet",
  deepgram: "Deepgram",
};

const PROVIDER_SHORT: Record<string, string> = {
  local_whisper: "whisper",
  apple_speech_on_device: "apple speech",
  parakeet: "parakeet",
  deepgram: "deepgram",
};

export function transcriptModelLabel(
  provider: string,
  modelId: string | null,
): TranscriptModelLabel {
  const name = PROVIDER_NAME[provider] ?? provider;
  // Parakeet ids spell out the whole architecture ("parakeet-tdt-0.6b-v3-onnx");
  // the version (plus int8 when quantized) is what tells two installs apart.
  const tail =
    provider === "parakeet" && modelId
      ? [/\bv\d+\b/.exec(modelId)?.[0], modelId.includes("int8") ? "int8" : null]
          .filter(Boolean)
          .join(" ")
      : modelId;
  return {
    label: [PROVIDER_SHORT[provider] ?? provider, tail].filter(Boolean).join(" "),
    tip: modelId ? `${name} · ${modelId}` : name,
  };
}
