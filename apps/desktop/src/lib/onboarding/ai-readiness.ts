// Onboarding AI readiness (issue #195, slice 11).
//
// The one rule: a default model counts ONLY if its provider verified LIVE this
// session and that live listing still contains that exact model id. Production's
// test — "a string reached the keychain" — is deliberately not enough, and no
// kind is exempt: a local Ollama that is not answering fails the same way a
// rejected cloud key does. This is what stops onboarding finishing with AI
// switched on and nothing behind it (which lands straight on the job-runner bug
// where a job against an absent model dies in ~6 min and is never retried).
//
// Pure and dependency-free on purpose — `onboarding-ai.svelte.ts` holds the
// `$state` and calls in here, so the rule is bun-testable without a runtime.

/** The live result of one `verify_ai_provider` round trip, per instance id. */
export type AiVerification =
  | { status: "checking" }
  | { status: "live"; models: string[]; latencyMs: number }
  | { status: "error"; reason: string };

/** One connected provider instance, reduced to what the rule needs. */
export interface AiReadinessProvider {
  id: string;
  label: string;
}

export interface AiReadinessInput {
  providers: readonly AiReadinessProvider[];
  /** Keyed by provider instance id (ADR 0035) — absent = never verified. */
  verifications: Readonly<Record<string, AiVerification | undefined>>;
  defaultModel: { provider: string; model: string } | null;
}

/**
 * The short human reason AI is not usable, or `null` when it is. PRIMARY: the
 * boolean below is derived from it so the two can never drift.
 */
export function aiReadinessMissing(input: AiReadinessInput): string | null {
  const { providers, verifications, defaultModel } = input;
  if (providers.length === 0) return "Connect a reasoning engine to use Ask AI.";

  const live = providers.filter((p) => verifications[p.id]?.status === "live");
  if (!defaultModel || defaultModel.model.trim().length === 0) {
    return live.length > 0
      ? "Choose a default model."
      : "No engine has verified yet — nothing can answer.";
  }

  const provider = providers.find((p) => p.id === defaultModel.provider);
  if (!provider) return "Pick a default model from a connected engine.";

  const verification = verifications[provider.id];
  if (verification?.status === "live") {
    return verification.models.includes(defaultModel.model)
      ? null
      : `${provider.label} no longer lists ${defaultModel.model} — pick another.`;
  }
  if (verification?.status === "error") {
    return `${provider.label} is not answering — ${verification.reason}.`;
  }
  if (verification?.status === "checking") return `Verifying ${provider.label}…`;
  return `Verify ${provider.label} before finishing.`;
}

/** `true` only when a verified engine lists the chosen model right now. */
export function aiReadinessReady(input: AiReadinessInput): boolean {
  return aiReadinessMissing(input) === null;
}

/** Status word for a provider card: "3 models · 412 ms" / "not answering" / … */
export function aiVerificationWord(verification: AiVerification | undefined): string {
  switch (verification?.status) {
    case "live":
      return `${plural(verification.models.length, "model")} · ${verification.latencyMs} ms`;
    case "checking":
      return "checking…";
    case "error":
      return verification.reason;
    default:
      return "not tested";
  }
}

/** "1 model" / "3 models" — the count is summed from live listings, never pasted. */
export function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}
