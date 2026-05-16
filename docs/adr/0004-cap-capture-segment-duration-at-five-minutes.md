# Cap capture segment duration at five minutes

Mnema caps **Capture Segment Duration** at 5 minutes while allowing **Capture Sessions** to continue until stopped. This bounds per-segment finalization, frame-index size, and automatic **Scrub Preview Generation** to at most 300 one-second preview intervals, trading more segment files and rotations for predictable cache warming, recovery, and preview-generation behavior.
