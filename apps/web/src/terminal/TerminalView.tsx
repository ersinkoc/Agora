import { useEffect, useRef } from 'react';
import { inputBytes } from './frames';
import { type TerminalRenderer, XtermRenderer } from './renderer';
import type { TerminalSource } from './source';

// Binds a renderer to a source. Input goes straight to the source (it outranks output, ADR-0022); output is
// written as it arrives; the renderer is torn down with the component.
export function TerminalView({ source, label }: { source: TerminalSource; label: string }) {
  const host = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const renderer: TerminalRenderer = new XtermRenderer();
    renderer.mount(el);
    const session = source.connect((e) => {
      if (e.type === 'data') renderer.write(e.bytes);
      else if (e.type === 'snapshot') renderer.reset(e.bytes);
      else if (e.type === 'resize') renderer.resize(e.cols, e.rows);
    });
    renderer.onInput((d) => session.sendInput(inputBytes(d)));
    const size = renderer.fit();
    if (size) session.resize(size.cols, size.rows);
    const observer = new ResizeObserver(() => {
      const s = renderer.fit();
      if (s) session.resize(s.cols, s.rows);
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
      session.close();
      renderer.dispose();
    };
  }, [source]); // The translated ARIA label can change without disposing the renderer or reconnecting the terminal.

  return <div ref={host} className="terminal" role="region" aria-label={label} dir="ltr" />;
}
