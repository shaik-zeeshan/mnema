# Spike finding: `wasapi` 0.23 capture-clock timestamps + peak-since-last-poll

Resolves [#51](https://github.com/shaik-zeeshan/mnema/issues/51). Verifies the two capabilities the
Windows microphone and system-audio capture work depends on before committing to `wasapi` 0.23 as the
binding (runtime-capture decision #2).

Source inspected: `wasapi-0.23.0` crate source as vendored in the local cargo registry
(`api.rs`, `lib.rs`, `examples/`), not docs.rs prose. Line references are into `wasapi-0.23.0/src/api.rs`.

## 1. Capture-clock timestamps on captured buffers — **YES**

Both buffer-read methods return a `BufferInfo` alongside the PCM:

- `AudioCaptureClient::read_from_device(&mut [u8]) -> (u32, BufferInfo)` (api.rs:1745)
- `AudioCaptureClient::read_from_device_to_deque(&mut VecDeque<u8>) -> BufferInfo` (api.rs:1788)

`BufferInfo` (api.rs:1639) carries, per packet:

- `timestamp: u64` — "timestamp in 100-nanosecond units of the first frame that was read from the
  buffer". This is the `pu64QPCPosition` out-param of `IAudioCaptureClient::GetBuffer` (api.rs:1755–1763):
  the QueryPerformanceCounter value, in 100 ns units, at the moment the endpoint captured the first frame
  in the packet — a **QPC capture-clock timestamp**.
- `index: u64` — the device position (`pu64DevicePosition`) of the first frame: a monotonic sample-count
  cursor for ordering/gap detection.
- `flags: BufferFlags` (api.rs:1671) — decoded `AUDCLNT_BUFFERFLAGS_*`, including `timestamp_error`
  (timestamp unreliable for this packet) and `data_discontinuity` (a glitch/gap occurred before this
  packet). These let us detect and skip untrustworthy timestamps rather than silently anchoring to them.

This is the same wall-clock model the rest of the runtime uses: WGC screen frames are anchored on
`SystemRelativeTime`, also a QPC timestamp (see `runtime-capture-research.md`). Audio Segment timing can
therefore be anchored to the same QPC base as screen capture.

Secondary clock (not needed for the above, but available): `AudioClient::get_audioclock()` →
`AudioClock::get_position() -> (position, qpc)` (api.rs:1557), wrapping `IAudioClock::GetPosition`, gives a
device position plus the QPC at read time. Note: `get_audioclock` is documented as non-functional in the
process-loopback (per-app) activation mode (api.rs:660), but that mode is not the baseline system-audio
path — ordinary `eRender` loopback uses the normal capture client and `BufferInfo.timestamp` is unaffected.

## 2. Peak-since-last-poll level reads — **NO dedicated API; capability still satisfied via PCM**

`wasapi` 0.23 does **not** wrap any audio level meter. There is no `IAudioMeterInformation` binding and no
`GetPeakValue` / `GetChannelsPeakValues` surface anywhere in the crate (confirmed by reading `api.rs` and
grepping the full crate source — the only `peak`/`level` hits are `SPEAKER_*` channel-mask constants and
buffer-duration doc text). So the literal "peak-since-last-poll level read" API does **not** exist here.

**This does not block the binding**, because the design never depended on a hardware meter:

- The macOS path already computes the activity level in software from the captured sample buffer
  (`capture-writers::derive_audio_activity_level_from_sample_buf`) and accumulates a *window peak* across
  polls that is reset on read (`capture-microphone`'s `record_microphone_activity_window_peak` /
  `LAST_MICROPHONE_ACTIVITY_WINDOW_PEAK_LEVEL_BITS`). CONTEXT.md defines the **Audio Activity Sample** as a
  reading *derived from* the PCM, not a hardware meter value.
- `wasapi` delivers the **complete** PCM stream: every captured packet is drained through
  `read_from_device(_to_deque)`, with `get_next_packet_size()` (api.rs:1725) to drain all queued packets.
  The WASAPI capture buffer retains all frames until released, so frames between two activity polls are
  **not** dropped — we compute peak over everything drained since the last poll and keep the max.

So the gap the requirement guards against ("a quiet gap between polls is not missed") is structurally
avoided: we own the accumulation window over the full PCM, exactly mirroring the existing macOS
`record_microphone_activity_window_peak` logic. This is the same architecture, not a Windows-specific
workaround.

## Verdict

| Capability | Direct `wasapi` 0.23 API | Status |
| --- | --- | --- |
| Capture-clock timestamps on buffers | `BufferInfo.timestamp` (QPC, 100 ns) + `.index` + `.flags.timestamp_error` from `read_from_device(_to_deque)` | **Yes** |
| Peak-since-last-poll level read | none (no `IAudioMeterInformation`/`GetPeakValue` wrapper) | **No named API**, but capability satisfied by computing peak over full drained PCM, identical to the macOS Audio Activity Sample path |

**No escalation required.** Capability 1 is provided directly. Capability 2 has no dedicated meter API in
`wasapi` 0.23, but is not a missing capability for our design — peak-since-last-poll is computed from the
PCM stream `wasapi` fully exposes, consistent with how macOS already derives the Audio Activity Sample.
`wasapi` 0.23 remains viable as the Windows audio binding; slice 1a can proceed.

One thing to carry into 1a (not a blocker, just an implementation note): treat `BufferFlags`
(`timestamp_error`, `data_discontinuity`, `silent`) as first-class inputs — skip timestamp anchoring on
`timestamp_error`, and treat `silent` packets as zero-level rather than reading uninitialized PCM.
