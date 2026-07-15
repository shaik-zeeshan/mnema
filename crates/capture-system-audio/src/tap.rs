use std::panic::AssertUnwindSafe;

use capture_types::CaptureErrorResponse;
use cidre::core_audio::hardware::sub_tap_keys;
use cidre::{arc, cat, cf, core_audio as ca, dispatch, ns, os};

use crate::{aggregate_uid_pid, system_audio_aggregate_uid, LOG_PREFIX};

// cidre 0.15 exposes neither `AudioDeviceDestroyIOProcID` nor a start/stop pair
// whose RAII shape survives the required Stop → DestroyIOProc → DestroyAggregate
// → DestroyTap ordering, so the four calls are declared against the same
// CoreAudio framework cidre already links.
#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C-unwind" {
    fn AudioDeviceStart(device: u32, proc_id: Option<ca::DeviceIoProcId>) -> os::Status;
    fn AudioDeviceStop(device: u32, proc_id: Option<ca::DeviceIoProcId>) -> os::Status;
    fn AudioDeviceDestroyIOProcID(device: u32, proc_id: Option<ca::DeviceIoProcId>) -> os::Status;
    fn AudioHardwareDestroyAggregateDevice(device: u32) -> os::Status;
}

fn tap_error(context: &str, error: impl std::fmt::Debug) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "system_audio_tap_start_failed".to_string(),
        message: format!("{context}: {error:?}"),
    }
}

/// A single tap generation: process tap, its private aggregate device, and the
/// running IOProc. Dropping tears all three down in Core Audio's required order.
///
/// **Field order is the teardown order.** `Drop` below does Stop →
/// DestroyIOProc by hand; the rest is Rust dropping fields in declaration order,
/// which is the only thing making DestroyAggregate happen before DestroyTap.
/// `_io_block` and `_queue` sit between `aggregate` and `_tap` because that is
/// where they land, not for tidiness — grouping the `_`-prefixed fields together
/// silently inverts the two destroys, and nothing here fails when it does.
pub struct SystemAudioTapSession {
    asbd: cat::AudioStreamBasicDesc,
    proc_id: ca::DeviceIoProcId,
    aggregate: ca::AggregateDevice,
    _io_block: arc::R<ca::DeviceIoBlock>,
    _queue: arc::R<dispatch::Queue>,
    _tap: ca::TapGuard,
}

impl SystemAudioTapSession {
    pub fn start(
        excluded_process_object_ids: &[u32],
        mut on_samples: impl FnMut(&cat::AudioTimeStamp, &[cat::AudioBuf]) + Send + 'static,
    ) -> Result<Self, CaptureErrorResponse> {
        let excluded: Vec<arc::R<ns::Number>> = excluded_process_object_ids
            .iter()
            .copied()
            .map(ns::Number::with_u32)
            .collect();
        let excluded = ns::Array::from_slice_retained(&excluded);

        // The `...ButExcludeProcesses:` initializer sets `isExclusive` true, which
        // is what makes the list an exclusion rather than an allow-list. Nothing
        // below may touch it: flipping it inverts the tap to silence, without error.
        let mut tap_desc = ca::TapDesc::with_stereo_global_tap_excluding_processes(&excluded);
        tap_desc.set_mute_behavior(ca::TapMuteBehavior::Unmuted);
        tap_desc.set_private(true);
        // ponytail: the initializer already mints a per-instance UUID; only the
        // human-readable name is stamped. Set the UUID explicitly if a tap ever
        // needs to be recognised across processes.
        let instance = cf::Uuid::new().to_cf_string().to_string();
        tap_desc.set_name(Some(&ns::String::with_str(&format!(
            "Mnema System Audio {instance}"
        ))));

        let tap = tap_desc
            .create_process_tap()
            .map_err(|error| tap_error("create process tap", error))?;
        let asbd = tap
            .asbd()
            .map_err(|error| tap_error("read tap format", error))?;
        let tap_uid = tap
            .uid()
            .map_err(|error| tap_error("read tap uid", error))?;

        // Only the default output device joins the aggregate; pulling in a
        // device's input streams would drag the microphone TCC prompt along.
        let output_device = ca::System::default_output_device()
            .map_err(|error| tap_error("resolve default output device", error))?;
        let output_uid = output_device
            .uid()
            .map_err(|error| tap_error("read default output device uid", error))?;

        let aggregate_uid =
            cf::String::from_str(&system_audio_aggregate_uid(std::process::id(), &instance));
        let desc = aggregate_desc(&aggregate_uid, &output_uid, &tap_uid);
        let aggregate = ca::AggregateDevice::with_desc(&desc)
            .map_err(|error| tap_error("create aggregate device", error))?;
        let device_id = aggregate.as_ref().0 .0;

        // The dispatch queue must be non-nil: passing nil makes the IOProc
        // silently never fire.
        let queue = dispatch::Queue::serial_with_ar_pool();
        let mut io_block = ca::DeviceIoBlock::<1, 1>::new5(
            move |_now: &cat::AudioTimeStamp,
                  input_data: &cat::AudioBufList<1>,
                  input_time: &cat::AudioTimeStamp,
                  _output_data: &mut cat::AudioBufList<1>,
                  _output_time: &cat::AudioTimeStamp| {
                let buffers = unsafe {
                    std::slice::from_raw_parts(
                        input_data.buffers.as_ptr(),
                        input_data.number_buffers as usize,
                    )
                };
                let delivered =
                    std::panic::catch_unwind(AssertUnwindSafe(|| on_samples(input_time, buffers)));
                if delivered.is_err() {
                    capture_runtime::debug_log!("{LOG_PREFIX} sample callback panicked");
                }
            },
        );
        let proc_id = aggregate
            .create_io_proc_id_with_block(Some(&queue), &mut io_block)
            .map_err(|error| tap_error("create io proc", error))?;

        if let Err(error) = unsafe { AudioDeviceStart(device_id, Some(proc_id)).result() } {
            unsafe { AudioDeviceDestroyIOProcID(device_id, Some(proc_id)) };
            return Err(tap_error("start aggregate device", error));
        }

        capture_runtime::debug_log!(
            "{LOG_PREFIX} started tap generation: {} Hz, {} ch, excluding {} process object(s)",
            asbd.sample_rate,
            asbd.channels_per_frame,
            excluded_process_object_ids.len()
        );

        Ok(Self {
            asbd,
            proc_id,
            aggregate,
            _io_block: io_block,
            _queue: queue,
            _tap: tap,
        })
    }

    /// The tap's own format (`kAudioTapPropertyFormat`). Device-dependent, and it
    /// changes across rebuilds.
    pub fn asbd(&self) -> cat::AudioStreamBasicDesc {
        self.asbd
    }
}

impl Drop for SystemAudioTapSession {
    fn drop(&mut self) {
        // Stop → DestroyIOProc here; the field order below then destroys the
        // aggregate before the tap.
        let device_id = self.aggregate.as_ref().0 .0;
        let stopped = unsafe { AudioDeviceStop(device_id, Some(self.proc_id)).result() };
        let destroyed =
            unsafe { AudioDeviceDestroyIOProcID(device_id, Some(self.proc_id)).result() };
        capture_runtime::debug_log!(
            "{LOG_PREFIX} stopped tap generation (stop={stopped:?}, destroy_io_proc={destroyed:?})"
        );
    }
}

fn aggregate_desc(
    aggregate_uid: &cf::String,
    output_uid: &cf::String,
    tap_uid: &cf::String,
) -> arc::R<cf::DictionaryOf<cf::String, cf::Type>> {
    let sub_device = cf::DictionaryOf::with_keys_values(
        &[ca::sub_device_keys::uid()],
        &[output_uid.as_type_ref()],
    );
    let sub_device_list = cf::ArrayOf::from_slice(&[sub_device.as_ref()]);

    let sub_tap = cf::DictionaryOf::with_keys_values(
        &[sub_tap_keys::uid(), sub_tap_keys::drift_compensation()],
        &[
            tap_uid.as_type_ref(),
            cf::Boolean::value_true().as_type_ref(),
        ],
    );
    let tap_list = cf::ArrayOf::from_slice(&[sub_tap.as_ref()]);

    cf::DictionaryOf::with_keys_values(
        &[
            ca::aggregate_device_keys::name(),
            ca::aggregate_device_keys::uid(),
            ca::aggregate_device_keys::main_sub_device(),
            ca::aggregate_device_keys::sub_device_list(),
            ca::aggregate_device_keys::tap_list(),
            ca::aggregate_device_keys::tap_auto_start(),
            ca::aggregate_device_keys::is_private(),
            ca::aggregate_device_keys::is_stacked(),
        ],
        &[
            cf::str!(c"Mnema System Audio").as_type_ref(),
            aggregate_uid.as_type_ref(),
            output_uid.as_type_ref(),
            sub_device_list.as_type_ref(),
            tap_list.as_type_ref(),
            cf::Boolean::value_true().as_type_ref(),
            cf::Boolean::value_true().as_type_ref(),
            cf::Boolean::value_false().as_type_ref(),
        ],
    )
}

/// Raises the "Screen & System Audio Recording" TCC prompt by building a tap and
/// throwing it away.
///
/// This is the whole official permission API: the prompt fires when a tap is
/// first *read*, not when it is created, and nothing can ask whether it was
/// granted afterwards (ADR 0052). So onboarding's Grant button runs a real tap
/// for a moment and discards whatever it hears — the point is the prompt, not
/// the audio. Nothing is excluded and nothing is written, and the delivery never
/// reaches [`crate::activity`], so this cannot feed the denial heuristic: a user
/// pressing Grant on a quiet Mac must not be judged for the silence.
///
/// Blocking (Core Audio start/teardown), and `Ok` only means the tap ran — on a
/// denied grant it runs perfectly and delivers zeros.
pub fn prompt_for_system_audio_permission() -> Result<(), CaptureErrorResponse> {
    let tap = SystemAudioTapSession::start(&[], |_, _| {})?;
    // The prompt is raised by the read the IOProc does, so give it one cycle to
    // happen before the tap goes away underneath it.
    std::thread::sleep(std::time::Duration::from_millis(250));
    drop(tap);
    Ok(())
}

/// Destroys aggregate devices minted by an earlier Mnema process that crashed
/// before its own teardown ran; their UIDs would otherwise collide with ours
/// (`AudioHardwareCreateAggregateDevice` fails with 1852797029).
pub fn cleanup_stale_aggregate_devices() -> usize {
    let own_pid = std::process::id();
    let Ok(devices) = ca::System::devices() else {
        return 0;
    };

    let mut destroyed = 0;
    for device in devices {
        let Ok(uid) = device.uid() else { continue };
        let uid = uid.to_string();
        if aggregate_uid_pid(&uid).is_none_or(|pid| pid == own_pid) {
            continue;
        }

        match unsafe { AudioHardwareDestroyAggregateDevice(device.0 .0).result() } {
            Ok(()) => {
                destroyed += 1;
                capture_runtime::debug_log!("{LOG_PREFIX} destroyed stale aggregate {uid}");
            }
            Err(error) => {
                capture_runtime::debug_log!(
                    "{LOG_PREFIX} failed to destroy stale aggregate {uid}: {error:?}"
                );
            }
        }
    }
    destroyed
}
