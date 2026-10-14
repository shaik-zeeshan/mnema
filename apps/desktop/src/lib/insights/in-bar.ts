/** Moves a view's controls onto the Insights bar row (insights.html `.in-bar__ctl`);
 *  the view keeps owning their state. Removed with the view. Never put it on the first
 *  top-level node of a component/block that has siblings: teardown walks siblings
 *  from the moved node and leaves the rest of the view mounted. */
export function inBar(node: HTMLElement): { destroy: () => void } {
  document.querySelector(".in-bar__ctl")?.append(node);
  return { destroy: () => node.remove() };
}
