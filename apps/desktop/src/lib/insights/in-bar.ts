/** Moves a view's controls onto the Insights bar row (insights.html `.in-bar__ctl`);
 *  the view keeps owning their state. Removed with the view. */
export function inBar(node: HTMLElement): { destroy: () => void } {
  document.querySelector(".in-bar__ctl")?.append(node);
  return { destroy: () => node.remove() };
}
