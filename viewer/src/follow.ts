// Keeping the newest turn of the conversation in view. A reply's code loads after it is shown, and every earlier
// reply's code loads again when the panel reopens, so the height keeps changing after the first scroll: the panel
// keeps following the newest turn until the person scrolls it themselves.

/** Where to scroll so the newest turn starts at the top of the view (the browser clamps it, so a short one sits at the bottom). */
export function followTarget(scrollTop: number, viewTop: number, turnTop: number, gap = 12): number {
  return Math.max(0, scrollTop + (turnTop - viewTop) - gap);
}

/** Whether the newest turn starts below what's on screen, i.e. the person is reading something older. */
export function latestBelow(viewBottom: number, turnTop: number, margin = 40): boolean {
  return turnTop > viewBottom - margin;
}
