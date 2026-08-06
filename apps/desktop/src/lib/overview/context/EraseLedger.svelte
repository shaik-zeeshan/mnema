<script lang="ts">
  // THE ERASE LEDGER — the one thing the Context destination must not get wrong.
  //
  // Every delete control in Mnema does something different to this page, and
  // none of the obvious ones clear it. Nothing on this table is a quantity, so
  // it is a TABLE and stays a table: no gauge, no bar, no instrument (the
  // direction's rule — an instrument names a physical quantity, and "what a
  // delete path touches" is not one).
  //
  // Every claim below was checked against the backend, not against the mockup
  // (verified 2026-08-06):
  //   · Retention Cleanup — `crates/app-infra/src/capture_retention.rs:477`
  //     run_cleanup_with_mode. Touches no `user_context_*` table at all;
  //     migration `0022_user_context_activities.sql:47` deliberately omits the
  //     FK to frames so derived rows SURVIVE aging (ADR 0029), and
  //     `capture_retention.rs:2457` is the test that pins it. It also deletes
  //     conversations on the same cutoff (`:498`, `:873` DELETE FROM
  //     conversations WHERE last_activity_at_ms < ?1) and whole
  //     `capture_segments`, not just frames and audio.
  //   · Delete Recent Capture — `crates/app-infra/src/user_context/store.rs:2554`
  //     cascade_derived_for_deleted_subjects_in, atomic with the raw delete.
  //     Drops Activities citing deleted evidence (`:2560`), digests overlapping
  //     them (`:2594`), then re-applies the formation bar (`:2633`):
  //     support-evidence count < (pinned ? 1 : FORMATION_BAR_EVIDENCE = 2,
  //     `user_context/confidence.rs:24`). `user_context_authored` and
  //     `user_context_dismissals` are explicitly left alone (`:2472`), with a
  //     test at `store.rs:5570`.
  //   · Wipe User Context — `apps/desktop/src-tauri/src/user_context/commands.rs:803`.
  //     Engine off FIRST (`:814`), then all 10 `user_context_*` tables
  //     (`store.rs:2499`) including authored + dismissals, then
  //     `conversation/store.rs:409` for every saved chat.
  interface Row {
    control: string;
    where: string;
    statements: string;
    conclusions: string;
    /** `true` when the column reads as a loss (danger), `false` when kept. */
    statementsLost: boolean;
    conclusionsLost: boolean;
    also: string;
  }

  // Two lines say more than the mockup drew, because the code does more:
  // retention also takes whole capture segments (not just frames and audio),
  // and Delete Recent Capture also takes the digests written over the deleted
  // span.
  const rows: Row[] = [
    {
      control: "Retention Cleanup",
      where: "“keep captures for 90 days”",
      statements: "kept",
      statementsLost: false,
      conclusions: "kept",
      conclusionsLost: false,
      also: "Deletes frames, audio and their capture segments past the window — and saved chats, which age out on the same cutoff.",
    },
    {
      control: "Delete Recent Capture",
      where: "the privacy panic button",
      statements: "kept",
      statementsLost: false,
      conclusions: "partly removed",
      conclusionsLost: true,
      also: "Drops the Activities that cited the deleted footage and any digest covering them, then any conclusion left with fewer than two supporting Activities. A pinned one survives down to its last support. The dismissed archive is untouched.",
    },
    {
      control: "Wipe User Context",
      where: "Settings › Intelligence",
      statements: "erased",
      statementsLost: true,
      conclusions: "erased",
      conclusionsLost: true,
      also: "Turns the engine off first, then clears every derived table, your authored statements, the dismissed archive and all saved chats.",
    },
  ];

</script>

<table class="ledger">
  <thead>
    <tr>
      <th class="ledger__c1">Control</th>
      <th class="ledger__c2">Your statements</th>
      <th class="ledger__c3">Inferred conclusions</th>
      <th>Also</th>
    </tr>
  </thead>
  <tbody>
    {#each rows as row (row.control)}
      <tr>
        <td>
          <span class="t-ui strong">{row.control}</span><br /><span
            class="t-meta subtle">{row.where}</span
          >
        </td>
        <td
          ><span class={row.statementsLost ? "lost" : "kept"}
            >{row.statements}</span
          ></td
        >
        <td
          ><span class={row.conclusionsLost ? "lost" : "kept"}
            >{row.conclusions}</span
          ></td
        >
        <td>{row.also}</td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .ledger {
    width: 100%;
    border-collapse: collapse;
  }
  .ledger th,
  .ledger td {
    text-align: left;
    padding: var(--s-8) var(--s-12);
    border-bottom: var(--hairline) solid var(--app-border);
    vertical-align: top;
  }
  .ledger th {
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .ledger td {
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text);
  }
  .ledger tr:last-child td {
    border-bottom: 0;
  }
  .ledger__c1 {
    width: 178px;
  }
  .ledger__c2,
  .ledger__c3 {
    width: 132px;
  }
  .lost {
    color: var(--app-danger);
    font-weight: var(--w-medium);
  }
  .kept {
    color: var(--app-accent);
    font-weight: var(--w-medium);
  }
  .strong {
    color: var(--app-text-strong);
  }
  .subtle {
    color: var(--app-text-subtle);
  }
</style>
