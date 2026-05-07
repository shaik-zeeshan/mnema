# Local-only audio transcription with app-managed models

Audio transcription will use local-only providers for v1: local Whisper, Apple on-device speech recognition, and Parakeet. App-managed models are stored outside the recording save directory and downloaded through the Tauri app layer from a versioned manifest, because transcription should preserve audio privacy while still giving users clear install status, progress, and recovery/backfill when models become available.
