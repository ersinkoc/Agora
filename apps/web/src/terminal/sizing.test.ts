import { describe, expect, it } from 'vitest';
import { type Size, TerminalSizing } from './sizing';

// A window and the hub's "smallest" rule (docs/specs/protocol.md "Shared terminal size"), wired the way they are in the app:
// the window's report goes to the hub, the hub resizes the node, the node's resize frame comes back to the window.
function harness(viewport: Size, others: Size[] = []) {
  const reports: Size[] = [];
  const shown: Size[] = [];
  const state = { viewport };
  const sizing: TerminalSizing = new TerminalSizing({
    measure: () => ({ ...state.viewport }),
    show: (s) => {
      shown.push(s);
      // Showing a terminal must not change what fits the viewport; even if a browser fired the observer for it, nothing is sent.
      sizing.viewportChanged();
    },
    report: (s) => {
      reports.push(s);
      const all = [s, ...others];
      const min = { cols: Math.min(...all.map((x) => x.cols)), rows: Math.min(...all.map((x) => x.rows)) };
      sizing.resized(min);
    },
  });
  return { sizing, reports, shown, state };
}

describe('shared terminal size', () => {
  it('reports what fits once and shows the smaller size the node chose without reporting it back', () => {
    const h = harness({ cols: 200, rows: 50 }, [{ cols: 100, rows: 30 }]);
    h.sizing.viewportChanged();
    expect(h.reports).toEqual([{ cols: 200, rows: 50 }]);
    expect(h.shown).toEqual([{ cols: 100, rows: 30 }]);
    expect(h.sizing.smaller()).toBe(true);
    // More resize frames of the same size (a snapshot's) and observer callbacks with the same viewport send nothing.
    h.sizing.resized({ cols: 100, rows: 30 });
    h.sizing.viewportChanged();
    h.sizing.viewportChanged();
    expect(h.reports).toHaveLength(1);
    expect(h.shown).toHaveLength(1);
  });

  it('reports again only when the viewport really changes', () => {
    const h = harness({ cols: 120, rows: 40 });
    h.sizing.viewportChanged();
    expect(h.shown).toEqual([{ cols: 120, rows: 40 }]);
    expect(h.sizing.smaller()).toBe(false);
    h.state.viewport = { cols: 90, rows: 40 };
    h.sizing.viewportChanged();
    expect(h.reports).toEqual([{ cols: 120, rows: 40 }, { cols: 90, rows: 40 }]);
    expect(h.shown.at(-1)).toEqual({ cols: 90, rows: 40 });
  });

  it('shows a bigger size set elsewhere as it is and does not fight it', () => {
    const h = harness({ cols: 80, rows: 24 });
    h.sizing.viewportChanged();
    h.sizing.resized({ cols: 160, rows: 48 }); // a terminal client on the node's own machine
    h.sizing.viewportChanged();
    expect(h.reports).toHaveLength(1);
    expect(h.shown.at(-1)).toEqual({ cols: 160, rows: 48 });
  });

  it('sends nothing while the viewport cannot be measured', () => {
    const reports: Size[] = [];
    const s = new TerminalSizing({ measure: () => undefined, show: () => {}, report: (z) => reports.push(z) });
    s.viewportChanged();
    expect(reports).toEqual([]);
  });
});
