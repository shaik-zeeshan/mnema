//! Native audio playback for the AudioDrawer: an `AVAudioPlayer` on the
//! segment's own file (no base64, no webview). AVFAudio has no objc2 crate in
//! our tree, so the handful of calls go through `msg_send!` on the class looked
//! up at runtime (AVFoundation, already linked by the capture crates, re-exports
//! AVFAudio).

use std::path::Path;

use objc2::msg_send;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSString, NSURL};

use crate::timeline::audio::transport::Player;

#[link(name = "AVFAudio", kind = "framework")]
extern "C" {}

pub struct AudioPlayer(Retained<AnyObject>);

// SAFETY: the player is created on a background thread (opening reads the file)
// and moved to the main thread once; from then on only the main thread touches it.
unsafe impl Send for AudioPlayer {}

impl AudioPlayer {
    /// Open `path` and prepare it (buffers loaded, duration known).
    pub fn open(path: &Path) -> Result<Self, String> {
        let unreadable = || "Couldn't open this segment's audio file.".to_string();
        let class = AnyClass::get(c"AVAudioPlayer").ok_or_else(unreadable)?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: plain AVAudioPlayer messages with their documented signatures;
        // a nil error out-pointer is allowed.
        let player: Option<Retained<AnyObject>> = unsafe {
            let alloc: Allocated<AnyObject> = msg_send![class, alloc];
            msg_send![alloc, initWithContentsOfURL: &*url, error: std::ptr::null_mut::<*mut AnyObject>()]
        };
        let player = player.ok_or_else(unreadable)?;
        let ready: Bool = unsafe { msg_send![&*player, prepareToPlay] };
        if !ready.as_bool() {
            return Err(unreadable());
        }
        Ok(Self(player))
    }
}

impl Player for AudioPlayer {
    fn play(&mut self) -> bool {
        let ok: Bool = unsafe { msg_send![&*self.0, play] };
        ok.as_bool()
    }
    fn pause(&mut self) {
        let _: () = unsafe { msg_send![&*self.0, pause] };
    }
    fn seek(&mut self, seconds: f64) {
        let _: () = unsafe { msg_send![&*self.0, setCurrentTime: seconds] };
    }
    fn position(&self) -> f64 {
        unsafe { msg_send![&*self.0, currentTime] }
    }
    fn duration(&self) -> f64 {
        unsafe { msg_send![&*self.0, duration] }
    }
    fn playing(&self) -> bool {
        let on: Bool = unsafe { msg_send![&*self.0, isPlaying] };
        on.as_bool()
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        // Closing the drawer mid-playback must not leave the sound running.
        self.pause();
    }
}
