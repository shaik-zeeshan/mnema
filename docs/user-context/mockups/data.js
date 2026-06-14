// data.js — the single shared dataset for the Subjects-Graph mockups.
//
// These 5 mockups redesign the Insights "Subjects" tab as one interactive
// connected graph. Rather than each mockup carrying its own copy of the data,
// they all read `window.MNEMA_GRAPH` from this file, so the same subjects,
// conclusions, and evidence-sharing edges render identically across every view.
//
// Shape (see the per-field comments below):
//   categories[] — the fixed taxonomy buckets, each mapped to a tokens.css color.
//   subjects[]   — what the engine has concluded about the user, grouped by category.
//   edges[]      — undirected links between subjects that share supporting evidence.
//
// Plain-language note: a "subject" is something the user keeps returning to; a
// "conclusion" is a short, evidence-backed thing the engine believes about it.
// Dependency-free — just assigns a global. No build step, no imports.

window.MNEMA_GRAPH = {
  // category key -> {label, token}. token is a CSS custom property in tokens.css.
  categories: [
    { key: "creating",      label: "Creating",      token: "--cat-creating" },
    { key: "research",      label: "Research",      token: "--cat-research" },
    { key: "learning",      label: "Learning",      token: "--cat-learning" },
    { key: "personal",      label: "Personal",      token: "--cat-personal" },
  ],
  subjects: [
    // id, name, category(key), conclusionCount, confidence(0..1 top conclusion),
    // trend("up"|"steady"|"down"), faded(bool), pinned(bool), headline,
    // lastMoved(label string e.g. "2h ago"), recencyDays(number, for timeline x),
    // conclusions: [{statement, confidence(0..1), status("visible"|"faded")}]
    {
      id:"tauri", name:"Tauri", category:"creating", conclusionCount:6, confidence:0.88,
      trend:"up", faded:false, pinned:true,
      headline:"Primary desktop stack for shipping Mnema",
      lastMoved:"12m ago", recencyDays:0.2,
      conclusions:[
        { statement:"Builds Mnema on Tauri as the main desktop stack", confidence:0.88, status:"visible" },
        { statement:"Wires Rust commands to the Svelte UI through invoke()", confidence:0.80, status:"visible" },
        { statement:"Chose Tauri over native Swift for cross-platform reach", confidence:0.72, status:"visible" },
      ],
    },
    {
      id:"privacy", name:"Privacy architecture", category:"creating", conclusionCount:4, confidence:0.80,
      trend:"up", faded:false, pinned:true,
      headline:"Local-first; secrets live only in the keychain",
      lastMoved:"1h ago", recencyDays:0.5,
      conclusions:[
        { statement:"Keeps capture and processing local-first by default", confidence:0.80, status:"visible" },
        { statement:"Stores API keys only in the OS keychain, never in config", confidence:0.76, status:"visible" },
        { statement:"Treats delete-capture as stronger than retention cleanup", confidence:0.63, status:"visible" },
      ],
    },
    {
      id:"diariz", name:"Diarization", category:"research", conclusionCount:5, confidence:0.69,
      trend:"up", faded:false, pinned:false,
      headline:"Speaker clustering is the DER ceiling",
      lastMoved:"3h ago", recencyDays:0.8,
      conclusions:[
        { statement:"Sees speaker clustering as the main limit on accuracy", confidence:0.69, status:"visible" },
        { statement:"Benchmarks diarization error rate against VoxConverse", confidence:0.61, status:"visible" },
        { statement:"Tunes clustering thresholds to curb over-splitting speakers", confidence:0.54, status:"visible" },
      ],
    },
    {
      id:"silicon", name:"Apple silicon", category:"research", conclusionCount:5, confidence:0.82,
      trend:"up", faded:false, pinned:false,
      headline:"Prefers M-series for local ML workloads",
      lastMoved:"5h ago", recencyDays:1,
      conclusions:[
        { statement:"Prefers M-series Macs for running local ML models", confidence:0.82, status:"visible" },
        { statement:"Leans on the unified memory for larger on-device inference", confidence:0.70, status:"visible" },
        { statement:"Profiles workloads against Apple's Neural Engine limits", confidence:0.58, status:"visible" },
      ],
    },
    {
      id:"sleep", name:"Sleep schedule", category:"personal", conclusionCount:3, confidence:0.55,
      trend:"down", faded:false, pinned:false,
      headline:"Late-night coding pushes bedtime past 2am",
      lastMoved:"6h ago", recencyDays:1,
      conclusions:[
        { statement:"Late-night coding sessions push bedtime past 2am", confidence:0.55, status:"visible" },
        { statement:"Wakes later on days after long evening work", confidence:0.46, status:"visible" },
      ],
    },
    {
      id:"llmeval", name:"LLM evaluation", category:"research", conclusionCount:4, confidence:0.66,
      trend:"up", faded:false, pinned:false,
      headline:"Adversarial verification beats single-vote",
      lastMoved:"1d ago", recencyDays:1.5,
      conclusions:[
        { statement:"Trusts adversarial verification over a single model vote", confidence:0.66, status:"visible" },
        { statement:"Cross-checks model outputs against cited evidence", confidence:0.57, status:"visible" },
        { statement:"Tracks where confident answers turn out wrong", confidence:0.49, status:"visible" },
      ],
    },
    {
      id:"rust", name:"Rust async", category:"learning", conclusionCount:4, confidence:0.74,
      trend:"up", faded:false, pinned:false,
      headline:"Comfortable with tokio; still wary of pinning",
      lastMoved:"2d ago", recencyDays:2,
      conclusions:[
        { statement:"Comfortable building async services on tokio", confidence:0.74, status:"visible" },
        { statement:"Still cautious around Pin and self-referential futures", confidence:0.60, status:"visible" },
        { statement:"Reaches for channels before shared locks", confidence:0.52, status:"visible" },
      ],
    },
    {
      id:"coffee", name:"Coffee intake", category:"personal", conclusionCount:2, confidence:0.47,
      trend:"steady", faded:false, pinned:false,
      headline:"~3 cups/day, clustered before noon",
      lastMoved:"2d ago", recencyDays:2,
      conclusions:[
        { statement:"Drinks about three cups of coffee a day", confidence:0.47, status:"visible" },
        { statement:"Most coffee happens before noon", confidence:0.41, status:"visible" },
      ],
    },
    {
      id:"designsys", name:"Design systems", category:"creating", conclusionCount:4, confidence:0.71,
      trend:"steady", faded:false, pinned:false,
      headline:"Terminal-green tokens drive every surface",
      lastMoved:"3d ago", recencyDays:3,
      conclusions:[
        { statement:"Drives every surface from shared design tokens", confidence:0.71, status:"visible" },
        { statement:"Anchors the palette on a terminal-green accent", confidence:0.62, status:"visible" },
        { statement:"Builds light and dark from the same token set", confidence:0.55, status:"visible" },
      ],
    },
    {
      id:"postgres", name:"Postgres", category:"learning", conclusionCount:3, confidence:0.58,
      trend:"steady", faded:false, pinned:false,
      headline:"sqlx migrations; comfortable with indexes",
      lastMoved:"4d ago", recencyDays:4,
      conclusions:[
        { statement:"Manages schema changes through sqlx migrations", confidence:0.58, status:"visible" },
        { statement:"Comfortable choosing indexes for common queries", confidence:0.50, status:"visible" },
      ],
    },
    {
      id:"climbing", name:"Climbing", category:"personal", conclusionCount:2, confidence:0.60,
      trend:"up", faded:false, pinned:false,
      headline:"Working a V4 project at the local gym",
      lastMoved:"5d ago", recencyDays:5,
      conclusions:[
        { statement:"Projecting a V4 boulder at the local gym", confidence:0.60, status:"visible" },
        { statement:"Climbs a few evenings most weeks", confidence:0.48, status:"visible" },
      ],
    },
    {
      id:"investing", name:"Investing", category:"personal", conclusionCount:3, confidence:0.52,
      trend:"steady", faded:false, pinned:false,
      headline:"Index-first, with occasional single-stock curiosity",
      lastMoved:"7d ago", recencyDays:7,
      conclusions:[
        { statement:"Favors broad index funds as the core approach", confidence:0.52, status:"visible" },
        { statement:"Occasionally researches a single stock out of curiosity", confidence:0.44, status:"visible" },
        { statement:"Prefers low-fee, long-hold positions", confidence:0.39, status:"visible" },
      ],
    },
    {
      id:"swiftui", name:"SwiftUI", category:"learning", conclusionCount:3, confidence:0.40,
      trend:"down", faded:true, pinned:false,
      headline:"Explored for native macOS; set aside for Tauri",
      lastMoved:"3w ago", recencyDays:21,
      conclusions:[
        { statement:"Explored SwiftUI for a native macOS build", confidence:0.40, status:"faded" },
        { statement:"Set it aside once Tauri covered the need", confidence:0.33, status:"faded" },
      ],
    },
    {
      id:"spanish", name:"Spanish", category:"learning", conclusionCount:2, confidence:0.35,
      trend:"down", faded:true, pinned:false,
      headline:"Duolingo streak has lapsed",
      lastMoved:"4w ago", recencyDays:30,
      conclusions:[
        { statement:"Was learning Spanish on Duolingo", confidence:0.35, status:"faded" },
        { statement:"The daily streak has lapsed for weeks", confidence:0.30, status:"faded" },
      ],
    },
  ],
  // undirected edges = subjects that share supporting evidence. weight 1..3 (3 = strongest)
  edges: [
    { a:"tauri", b:"rust", weight:3 }, { a:"tauri", b:"designsys", weight:3 }, { a:"tauri", b:"privacy", weight:3 },
    { a:"tauri", b:"postgres", weight:1 }, { a:"tauri", b:"swiftui", weight:1 },
    { a:"rust", b:"postgres", weight:2 }, { a:"rust", b:"silicon", weight:1 },
    { a:"diariz", b:"llmeval", weight:3 }, { a:"diariz", b:"silicon", weight:2 }, { a:"llmeval", b:"silicon", weight:2 },
    { a:"designsys", b:"privacy", weight:2 }, { a:"investing", b:"privacy", weight:1 },
    { a:"sleep", b:"coffee", weight:2 }, { a:"sleep", b:"climbing", weight:1 }, { a:"coffee", b:"climbing", weight:1 },
  ],
};
