# Support an opt-in Preview Update channel

Mnema V1 supports a default **Stable Update** channel and an opt-in **Preview Update** channel. Preview updates exist because Developer ID signing and notarization timing is uncertain, but users still need a deliberate way to receive prerelease builds; preview must be clearly labeled as less stable and may show macOS security warnings until the release pipeline has Developer ID signing and notarization.

Preview uses explicit prerelease versions such as `0.3.0-preview.1` and a GitHub Pages feed generated only during manual release promotion. Stable remains backed by the latest published non-prerelease GitHub Release.
