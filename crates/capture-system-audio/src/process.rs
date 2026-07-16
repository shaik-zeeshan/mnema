use capture_types::CaptureErrorResponse;
use cidre::{core_audio as ca, ns};

/// Translates our own pid to its Core Audio process object via
/// `kAudioHardwarePropertyTranslatePIDToProcessObject`, for callers building a
/// tap exclude list.
///
/// `Ok(None)` is the one benign absence — Core Audio has not minted an object
/// for our pid yet — and it self-heals on the next reconcile. A failed read is
/// an `Err` rather than a `None`, because the two are not the same fact: read
/// as "not yet minted" it would drop self-exclusion from the list, rebuild the
/// tap without it, and rebuild again once the next read succeeds.
pub fn own_process_object_id() -> Result<Option<u32>, CaptureErrorResponse> {
    let pid = ns::ProcessInfo::current().process_id();
    let process = ca::Process::with_pid(pid).map_err(|error| CaptureErrorResponse {
        code: "system_audio_exclude_list_failed".to_string(),
        message: format!("translate own pid to audio process object: {error:?}"),
    })?;
    let object_id = process.0 .0;
    Ok((object_id != ca::Obj::UNKNOWN.0).then_some(object_id))
}
