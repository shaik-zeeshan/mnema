# Secret Redaction V2 is prospective

Mnema will apply **Secret Redaction Pipeline** V2 to newly completed OCR/transcript derived text and to results the user explicitly reprocesses, but it will not automatically mutate existing V1-derived results during upgrade. This keeps upgrades non-surprising, avoids large background scanner work and OCR/transcription reruns, and preserves the current history unless the user asks Mnema to reprocess it; the trade-off is that V1-era false negatives can remain searchable, copyable, or broker-visible until explicitly reprocessed or deleted.
