// One terminal may be watched from several windows at once. The hub gives it the smallest size any of them asked for (tmux's
// "smallest", docs/specs/protocol.md "Shared terminal size"), so no window is clipped; a bigger window shows it at its top left
// with the pane's surface around it.
//
// A window therefore reports the size that fits its viewport and shows the size the node reports (the terminal's resize frame).
// It reports again only when what fits its viewport changes, never because the terminal's size changed: otherwise a terminal
// smaller than the viewport would be reported, shrunk, reported again, and so on.

export interface Size {
  cols: number;
  rows: number;
}

export interface SizingHooks {
  /** What fits the viewport now, without changing the terminal (undefined while it cannot be measured). */
  measure(): Size | undefined;
  /** Show the terminal at the node's size. */
  show(size: Size): void;
  /** Tell the node (through the hub) what fits this window. */
  report(size: Size): void;
}

const same = (a: Size | undefined, b: Size | undefined) => !!a && !!b && a.cols === b.cols && a.rows === b.rows;

export class TerminalSizing {
  private reported: Size | undefined;
  private shown: Size | undefined;

  constructor(private readonly hooks: SizingHooks) {}

  /** The viewport, the font or the theme changed (or the terminal was just mounted). */
  viewportChanged(): void {
    const m = this.hooks.measure();
    if (!m || same(m, this.reported)) return;
    this.reported = m;
    this.hooks.report(m);
  }

  /** The node says the terminal now has this size. Shown as it is; never reported back. */
  resized(size: Size): void {
    if (same(size, this.shown)) return;
    this.shown = size;
    this.hooks.show(size);
  }

  /** Whether the terminal is smaller than this window in either direction (a smaller window watches it too). */
  smaller(): boolean {
    const s = this.shown;
    const r = this.reported;
    return !!s && !!r && (s.cols < r.cols || s.rows < r.rows);
  }
}
