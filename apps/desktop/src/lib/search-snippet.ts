export type SearchSnippetSegment = { text: string; marked: boolean };

export function parseSearchSnippet(snippet: string): SearchSnippetSegment[] {
  const segments: SearchSnippetSegment[] = [];
  let index = 0;
  let marked = false;
  while (index < snippet.length) {
    const nextOpen = snippet.indexOf("<mark>", index);
    const nextClose = snippet.indexOf("</mark>", index);
    const next =
      nextOpen >= 0 && (nextClose < 0 || nextOpen < nextClose)
        ? { at: nextOpen, tag: "<mark>", markedAfter: true }
        : nextClose >= 0
          ? { at: nextClose, tag: "</mark>", markedAfter: false }
          : null;
    if (!next) {
      if (index < snippet.length) segments.push({ text: snippet.slice(index), marked });
      break;
    }
    if (next.at > index) segments.push({ text: snippet.slice(index, next.at), marked });
    marked = next.markedAfter;
    index = next.at + next.tag.length;
  }
  return segments;
}
