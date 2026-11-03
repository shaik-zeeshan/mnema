// Data shape of a comparison page (src/layouts/Compare.astro renders it).

export interface FieldTable {
  /** Column 0 is Mnema. */
  columns: string[];
  rows: { label: string; cells: string[] }[];
  note: string;
}

interface SectionBase {
  id: string;
  /** Label in the "On this page" rail. */
  toc: string;
  kicker: string;
  title: string;
  intro?: string;
}

export type Section = SectionBase &
  (
    | { kind: "history"; rows: { when: string; text: string }[] }
    | { kind: "prose"; paras: string[] }
    | { kind: "criteria"; items: { label: string; detail: string }[] }
    | ({ kind: "field" } & FieldTable)
  );

export interface Faq {
  q: string;
  a: string;
}
