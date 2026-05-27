# Speaker Model Presets and Per-Preset Voiceprint Scope

We are opening speaker analysis from one fixed model to a small set of curated **Speaker Model Presets** while keeping the on-device sherpa-onnx CPU runtime. Each preset is one combined segmentation+embedding `model_id`; users pick intent ("Balanced", "Multilingual", "High-accuracy"), not raw model files. Recognition stays scoped to one **Voiceprint Space** per preset, and switching presets is non-destructive and warned, not blocked or migrated.

## Considered Options

- **Raw segmentation × embedding pickers** — rejected. The data model stores one combined `model_id` everywhere it matters (settings, manifest, `person_voice_embeddings.model_id`, Speaker Continuity keying, download orchestration). Splitting it into two dimensions is a large change that also lets users assemble untested, unvalidated model combinations.
- **Auto-migrating voiceprints on preset switch** — rejected. Re-embedding a saved person into a new embedding space needs the original enrollment *audio*, but the DB stores only the vector, and retention may have deleted the source. It is a fragile, large feature solving a problem the substrate already handles reversibly.
- **Blocking preset switches when people are enrolled** — rejected. It punishes the user for wanting multilingual support, which is the point of the feature.
- **Building the Apple Neural Engine provider (FluidAudio/CoreML) now** — deferred. It is the only honest answer to "faster / lower power," but it is a substantial Swift/CoreML integration. This round is CPU-only and does not claim a speed/power win.
- **Shipping reverb-v2 (242MB) as a max-accuracy preset** — deferred pending measurement of the v1→v2 gain on Mnema's actual audio, and of CPU time against the helper timeout. Adding it later is a pure manifest entry, no new code.

## Consequences

- A preset switch leaves diarization (who-spoke-when) working immediately; only recognition (naming saved voices) goes dormant until the user re-tags people once under the new preset. Switching back restores the prior preset's recognition, because enrollments persist per `model_id`.
- A mixed-preset library stays internally consistent: past recordings keep their original-preset results, and we do not re-run diarization on history when the preset changes.
- The settings switch surfaces a Tauri confirm dialog only when enrolled people exist; new users switch silently. Per-preset download size is shown up front.
- Per-model clustering/cross-chunk thresholds and minimum-turn-duration become descriptor fields so new presets are not stuck with values tuned for the original combo.
- "Faster / lower power" remains an unmet, explicitly parked goal; the future ANE/FluidAudio provider is its intended home and would slot in as another provider with its own presets.
