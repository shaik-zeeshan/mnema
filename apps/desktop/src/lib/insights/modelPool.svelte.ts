// Shared incremental model-pool loader for the model pickers (Chat composer pin
// + both Settings pickers, all rendered via <ModelPickerMenu>).
//
// Lists each connected provider's models with ONE `ai_runtime_list_models` call
// PER provider, in parallel, merging each provider's slice into `pool` the
// moment it resolves — so a fast provider's models show immediately instead of
// waiting on the slowest, and a slow/unreachable provider only delays its OWN
// slice (surfaced as a `failure` with a Retry) rather than blocking the whole
// list. The backend already times out an unresponsive provider per call, so the
// fan-out caps the wait at the slowest single provider, not their sum.
import { invoke } from "@tauri-apps/api/core";
import type {
  AiProviderConfig,
  AiRuntimeModel,
  AiRuntimeModelsResult,
  AiRuntimeProviderFailure,
} from "$lib/types/recording";

export class ModelPoolLoader {
  /** The merged provider-tagged pool, grown incrementally as providers resolve. */
  pool = $state<AiRuntimeModel[]>([]);
  /** True while ANY provider's listing is still in flight. */
  loading = $state(false);
  /** True once a full listing has completed at least once. */
  loaded = $state(false);
  /** Providers that failed to list last fetch (unreachable, missing key, …). */
  failures = $state<AiRuntimeProviderFailure[]>([]);

  /**
   * List the given providers, one call per provider in parallel, merging each
   * provider's result the moment it lands. Each provider's slice is replaced
   * wholesale, so a Retry of just the failed providers leaves the already-loaded
   * ones untouched. A concurrent call is a no-op (the in-flight one wins).
   */
  async load(targets: AiProviderConfig[]): Promise<void> {
    if (this.loading) return;
    if (targets.length === 0) {
      this.loaded = true;
      return;
    }
    // Snapshot so an in-progress settings draft can't mutate the list mid-flight.
    const providers = $state.snapshot(targets);
    this.loading = true;
    await Promise.all(
      providers.map(async (provider) => {
        let models: AiRuntimeModel[] = [];
        let failure: AiRuntimeProviderFailure | null = null;
        try {
          // A single-element provider list lists JUST this provider, so its
          // result arrives independently of the others.
          const result = await invoke<AiRuntimeModelsResult>("ai_runtime_list_models", {
            request: { providers: [provider] },
          });
          models = result.models;
          failure = result.failures[0] ?? null;
        } catch {
          failure = { provider: provider.id, reason: "couldn't list models" };
        }
        // Replace this provider's slice (incremental: triggers a re-render as
        // soon as THIS provider resolves, without waiting on its peers).
        this.pool = [
          ...this.pool.filter((m) => m.provider !== provider.id),
          ...models,
        ];
        this.failures = [
          ...this.failures.filter((f) => f.provider !== provider.id),
          ...(failure ? [failure] : []),
        ];
      }),
    );
    this.loading = false;
    this.loaded = true;
  }

  /** Drop the cached pool (the connected-provider set changed); the next load
   *  re-discovers against the current providers. */
  reset(): void {
    this.loaded = false;
    this.pool = [];
    this.failures = [];
  }
}
