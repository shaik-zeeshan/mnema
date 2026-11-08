//! The frame-artifact channel: the screen capture callback hands each exported
//! frame (with the metadata snapshot in effect when it was captured) to the
//! persistence worker over a bounded FIFO, and stop/rotation paths flush it.

use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameArtifactForwardingResult {
    Enqueued,
    ReceiverClosed,
}

pub enum FrameArtifactMessage {
    Artifact(FrameArtifactEnvelope),
    Flush(std::sync::mpsc::SyncSender<()>),
}

pub struct FrameArtifactEnvelope {
    pub artifact: capture_screen::ScreenFrameArtifact,
    pub metadata_snapshot: Option<capture_metadata::FrameMetadataSnapshot>,
}

#[cfg(test)]
impl FrameArtifactMessage {
    pub fn unwrap_artifact(self) -> capture_screen::ScreenFrameArtifact {
        match self {
            Self::Artifact(envelope) => envelope.artifact,
            Self::Flush(_) => panic!("expected Artifact, got Flush"),
        }
    }

    pub fn unwrap_artifact_envelope(self) -> FrameArtifactEnvelope {
        match self {
            Self::Artifact(envelope) => envelope,
            Self::Flush(_) => panic!("expected Artifact, got Flush"),
        }
    }
}

pub fn try_forward_frame_artifact(
    frame_artifact_tx: &mpsc::Sender<FrameArtifactMessage>,
    artifact: capture_screen::ScreenFrameArtifact,
    metadata_snapshot: Option<capture_metadata::FrameMetadataSnapshot>,
) -> FrameArtifactForwardingResult {
    match frame_artifact_tx.blocking_send(FrameArtifactMessage::Artifact(FrameArtifactEnvelope {
        artifact,
        metadata_snapshot,
    })) {
        Ok(()) => FrameArtifactForwardingResult::Enqueued,
        Err(_) => FrameArtifactForwardingResult::ReceiverClosed,
    }
}

/// Blocks until all frame artifacts currently in the queue have been persisted.
///
/// Sends a flush marker through the same FIFO channel and waits for the worker
/// to acknowledge it. Because the channel is ordered, every artifact enqueued
/// before the marker is guaranteed to be fully persisted when this returns.
///
/// Uses `try_send` with a brief retry so the call is safe from any thread
/// context (including tokio-associated threads where `blocking_send` would
/// panic).
pub fn flush_frame_artifacts(frame_artifact_tx: &mpsc::Sender<FrameArtifactMessage>) {
    let (response_tx, response_rx) = std::sync::mpsc::sync_channel(1);
    let mut message = FrameArtifactMessage::Flush(response_tx);
    loop {
        match frame_artifact_tx.try_send(message) {
            Ok(()) => break,
            Err(mpsc::error::TrySendError::Full(returned)) => {
                message = returned;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
        }
    }
    // 30 s is deliberately generous; the worker should ack almost immediately
    // after draining the buffered artifacts ahead of the flush marker.
    let _ = response_rx.recv_timeout(Duration::from_secs(30));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Tauri runtime's `block_on`, for the tests that drive the channel.
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Runtime::new()
            .expect("test runtime starts")
            .block_on(future)
    }

    #[test]
    fn try_forward_frame_artifact_enqueues_when_capacity_available() {
        let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(1);

        let result = try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-1.png".to_string(),
                captured_at_unix_ms: 1,
                width: Some(100),
                height: Some(100),
                captured_frame_equivalence: capture_screen::CapturedFrameEquivalenceOutcome::Ready(
                    capture_screen::CapturedFrameEquivalence {
                        hint: "hint-1".to_string(),
                        proof: b"proof-1".to_vec(),
                        version: capture_screen::CAPTURED_FRAME_EQUIVALENCE_VERSION,
                    },
                ),
            },
            None,
        );

        assert_eq!(result, FrameArtifactForwardingResult::Enqueued);

        let queued = rx
            .try_recv()
            .expect("frame should be queued")
            .unwrap_artifact();
        assert_eq!(queued.file_path, "/tmp/frame-1.png");
    }

    #[test]
    fn try_forward_frame_artifact_enqueues_metadata_snapshot_with_artifact() {
        let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(1);
        let snapshot = capture_metadata::FrameMetadataSnapshot {
            app_bundle_id: Some("com.example.App".to_string()),
            app_name: Some("Example".to_string()),
            window_title: Some("Original Window".to_string()),
            window_id: None,
            browser_url: None,
            display_id: None,
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        };

        let result = try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-1.png".to_string(),
                captured_at_unix_ms: 1,
                width: None,
                height: None,
                captured_frame_equivalence:
                    capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
            },
            Some(snapshot.clone()),
        );

        assert_eq!(result, FrameArtifactForwardingResult::Enqueued);

        let queued = rx
            .try_recv()
            .expect("frame should be queued")
            .unwrap_artifact_envelope();
        assert_eq!(queued.artifact.file_path, "/tmp/frame-1.png");
        assert_eq!(queued.metadata_snapshot, Some(snapshot));
    }

    #[test]
    fn try_forward_frame_artifact_enqueues_multiple_frames_without_dropping() {
        let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(2);

        let first = try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-1.png".to_string(),
                captured_at_unix_ms: 1,
                width: None,
                height: None,
                captured_frame_equivalence:
                    capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
            },
            None,
        );
        let second = try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-2.png".to_string(),
                captured_at_unix_ms: 2,
                width: None,
                height: None,
                captured_frame_equivalence:
                    capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
            },
            None,
        );

        assert_eq!(first, FrameArtifactForwardingResult::Enqueued);
        assert_eq!(second, FrameArtifactForwardingResult::Enqueued);

        let first = rx
            .try_recv()
            .expect("first frame should reach worker queue")
            .unwrap_artifact();
        let second = rx
            .try_recv()
            .expect("second frame should reach worker queue")
            .unwrap_artifact();
        assert_eq!(first.file_path, "/tmp/frame-1.png");
        assert_eq!(second.file_path, "/tmp/frame-2.png");
    }

    #[test]
    fn try_forward_frame_artifact_waits_for_capacity_without_dropping_frames() {
        let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(1);

        let first = try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-1.png".to_string(),
                captured_at_unix_ms: 1,
                width: None,
                height: None,
                captured_frame_equivalence:
                    capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
            },
            None,
        );

        assert_eq!(first, FrameArtifactForwardingResult::Enqueued);

        let sender = std::thread::spawn(move || {
            try_forward_frame_artifact(
                &tx,
                capture_screen::ScreenFrameArtifact {
                    file_path: "/tmp/frame-2.png".to_string(),
                    captured_at_unix_ms: 2,
                    width: None,
                    height: None,
                    captured_frame_equivalence:
                        capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
                },
                None,
            )
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !sender.is_finished(),
            "second send should wait for queue capacity"
        );

        let queued = block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .expect("first frame should arrive before timeout")
                .expect("first frame should be queued")
        })
        .unwrap_artifact();
        assert_eq!(queued.file_path, "/tmp/frame-1.png");

        let second = sender.join().expect("sender thread should exit cleanly");
        assert_eq!(second, FrameArtifactForwardingResult::Enqueued);

        let deferred = block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .expect("second frame should arrive before timeout")
                .expect("second frame should be queued")
        })
        .unwrap_artifact();
        assert_eq!(deferred.file_path, "/tmp/frame-2.png");
    }

    #[test]
    fn try_forward_frame_artifact_reports_closed_receiver() {
        let (tx, rx) = mpsc::channel::<FrameArtifactMessage>(1);
        drop(rx);

        let result = try_forward_frame_artifact(
            &tx,
            capture_screen::ScreenFrameArtifact {
                file_path: "/tmp/frame-1.png".to_string(),
                captured_at_unix_ms: 1,
                width: None,
                height: None,
                captured_frame_equivalence:
                    capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
            },
            None,
        );

        assert_eq!(result, FrameArtifactForwardingResult::ReceiverClosed);
    }

    #[test]
    fn flush_frame_artifacts_waits_for_all_queued_items() {
        let (tx, mut rx) = mpsc::channel::<FrameArtifactMessage>(4);

        // Enqueue two frame artifacts before the flush.
        for i in 1..=2 {
            tx.try_send(FrameArtifactMessage::Artifact(FrameArtifactEnvelope {
                artifact: capture_screen::ScreenFrameArtifact {
                    file_path: format!("/tmp/frame-{i}.png"),
                    captured_at_unix_ms: i,
                    width: None,
                    height: None,
                    captured_frame_equivalence:
                        capture_screen::CapturedFrameEquivalenceOutcome::quarantined("test"),
                },
                metadata_snapshot: None,
            }))
            .expect("channel should have capacity");
        }

        // Track which artifacts the consumer sees before the flush ack.
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_for_consumer = std::sync::Arc::clone(&seen);

        // Consumer task: drain messages, record artifacts, ack flushes.
        let consumer = std::thread::spawn(move || {
            block_on(async {
                while let Some(message) = rx.recv().await {
                    match message {
                        FrameArtifactMessage::Artifact(envelope) => {
                            // Simulate some processing latency.
                            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                            seen_for_consumer
                                .lock()
                                .expect("seen state should lock")
                                .push(envelope.artifact.file_path);
                        }
                        FrameArtifactMessage::Flush(response_tx) => {
                            let _ = response_tx.send(());
                        }
                    }
                }
            });
        });

        // Flush from another thread (simulating the segment loop / stop path).
        flush_frame_artifacts(&tx);

        // After flush returns, all prior artifacts must have been processed.
        let processed = seen.lock().expect("seen state should lock").clone();
        assert_eq!(
            processed,
            vec![
                "/tmp/frame-1.png".to_string(),
                "/tmp/frame-2.png".to_string()
            ],
            "flush must drain all artifacts enqueued before the barrier"
        );

        // Drop the sender so the consumer exits.
        drop(tx);
        consumer
            .join()
            .expect("consumer thread should exit cleanly");
    }

    #[test]
    fn flush_frame_artifacts_is_noop_when_channel_closed() {
        let (tx, rx) = mpsc::channel::<FrameArtifactMessage>(1);
        drop(rx);

        // Must not hang or panic when the receiver is already gone.
        flush_frame_artifacts(&tx);
    }
}
