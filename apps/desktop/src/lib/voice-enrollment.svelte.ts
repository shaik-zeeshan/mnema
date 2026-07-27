// Voice enrollment — the reactive half: one take loop over the four backend
// commands, shared by every enrollment door (Settings today; the onboarding
// Voice screen can construct the same store).
//
//   record_bounded_microphone_clip  → a temp .m4a (the live-session guard that
//                                     pauses/resumes the microphone around the
//                                     take is BACKEND-owned; nothing here
//                                     manages capture state)
//   get_microphone_activity_level   → the level meter, polled while recording
//   enroll_account_owner_voice      → enrolled, or one of three typed rejections
//   get_account_owner_person_id     → whether a voiceprint exists
//
// This store never judges a take: it renders whatever verdict the embedder
// returns and offers a retry. It also never flips `recognize_saved_people` —
// `enroll_account_owner_voice` does that backend-side.

import { invoke } from "@tauri-apps/api/core";
import { errorText } from "$lib/settings/state/format";
import type { PersonProfileDto } from "$lib/types/app-infra";
import {
  ENROLLMENT_CLIP_MS,
  type VoiceEnrollmentOutcome,
  type VoiceEnrollmentRejection,
} from "$lib/voice-enrollment";

export type EnrollmentStage = "idle" | "recording" | "review" | "enrolling";

export class VoiceEnrollmentStore {
  /** True until the first `load()` settles, so the card can hold its shape. */
  loading = $state(true);
  /** The account-owner profile, or null when no voiceprint exists. */
  owner = $state<PersonProfileDto | null>(null);
  stage = $state<EnrollmentStage>("idle");
  /** Latest microphone level, 0–1. Only meaningful while recording. */
  level = $state(0);
  secondsLeft = $state(0);
  clipPath = $state<string | null>(null);
  /** The embedder's last rejection, cleared when a new take starts. */
  rejection = $state<VoiceEnrollmentRejection | null>(null);
  /** A command failure (not a rejection) — mic busy, no build support, … */
  error = $state<string | null>(null);
  deleting = $state(false);
  /** Set for one render after a successful enroll, so the card can say so. */
  justEnrolled = $state(false);

  #levelTimer: ReturnType<typeof setInterval> | null = null;

  /** Read whether a voiceprint exists, and who it belongs to. */
  async load(): Promise<void> {
    this.loading = true;
    try {
      const personId = await invoke<number | null>("get_account_owner_person_id");
      if (personId === null || personId === undefined) {
        this.owner = null;
      } else {
        const profiles = await invoke<PersonProfileDto[]>("list_person_profiles");
        this.owner = profiles.find((p) => p.id === personId) ?? null;
      }
    } catch (err) {
      this.error = errorText(err);
      this.owner = null;
    } finally {
      this.loading = false;
    }
  }

  /** Take a fixed-length clip, polling the level meter while it runs. */
  async record(): Promise<void> {
    if (this.stage !== "idle") return;
    this.rejection = null;
    this.error = null;
    this.clipPath = null;
    this.justEnrolled = false;
    this.stage = "recording";
    this.secondsLeft = Math.ceil(ENROLLMENT_CLIP_MS / 1000);
    const startedAt = Date.now();
    this.#levelTimer = setInterval(() => {
      void invoke<number | null>("get_microphone_activity_level")
        .then((value) => {
          this.level = typeof value === "number" ? Math.min(1, Math.max(0, value)) : 0;
        })
        .catch(() => {
          this.level = 0;
        });
      const remaining = ENROLLMENT_CLIP_MS - (Date.now() - startedAt);
      this.secondsLeft = Math.max(0, Math.ceil(remaining / 1000));
    }, 150);
    try {
      this.clipPath = await invoke<string>("record_bounded_microphone_clip", {
        durationMs: ENROLLMENT_CLIP_MS,
      });
      this.stage = "review";
    } catch (err) {
      this.error = errorText(err);
      this.stage = "idle";
    } finally {
      this.#stopLevelPolling();
    }
  }

  /**
   * Submit the take. Returns true when a voiceprint was stored — the caller
   * mirrors the backend's `recognize_saved_people` flip into its own draft
   * settings so a later autosave cannot write the stale `false` back.
   */
  async enroll(displayName?: string): Promise<boolean> {
    if (this.stage !== "review" || !this.clipPath) return false;
    this.stage = "enrolling";
    this.error = null;
    try {
      const outcome = await invoke<VoiceEnrollmentOutcome>("enroll_account_owner_voice", {
        request: { clipPath: this.clipPath, displayName: displayName ?? null },
      });
      this.clipPath = null;
      this.stage = "idle";
      if (outcome.status === "enrolled") {
        this.owner = outcome.profile;
        this.rejection = null;
        this.justEnrolled = true;
        return true;
      }
      this.rejection = outcome;
      return false;
    } catch (err) {
      this.error = errorText(err);
      this.stage = "idle";
      return false;
    }
  }

  /** Throw the take away without submitting it. */
  discard(): void {
    if (this.stage !== "review") return;
    this.clipPath = null;
    this.stage = "idle";
  }

  /**
   * Delete the Person Profile. Voiceprints cascade (`ON DELETE CASCADE` with
   * `foreign_keys(true)`), so there is nothing else to delete. The caller owns
   * the confirmation dialog.
   */
  async deleteProfile(): Promise<void> {
    const personId = this.owner?.id;
    if (personId === undefined || this.deleting) return;
    this.deleting = true;
    this.error = null;
    try {
      await invoke("delete_person_profile", { request: { personId } });
      this.owner = null;
      this.rejection = null;
      this.justEnrolled = false;
    } catch (err) {
      this.error = errorText(err);
    } finally {
      this.deleting = false;
    }
  }

  /** Stop the meter poll — call from the host's `onDestroy`. */
  dispose(): void {
    this.#stopLevelPolling();
  }

  #stopLevelPolling(): void {
    if (this.#levelTimer !== null) clearInterval(this.#levelTimer);
    this.#levelTimer = null;
    this.level = 0;
    this.secondsLeft = 0;
  }
}
