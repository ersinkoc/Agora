// A development plugin's pane says when its changed file was not loaded (reload on save, devReload.ts): in the window's own words, above
// the frame and never inside it, while the last package that passed keeps running.
import { useDevReload } from './devReload';
import { reloadProblemText } from './PluginsDialog';

export function ReloadNote({ plugin, name }: { plugin: string; name: string }) {
  const p = useDevReload().problems[plugin];
  if (!p) return null;
  return <p role="status" data-plugin-reload-problem={plugin} className="m-0 flex-none border-b border-line bg-tint-danger px-3 py-1.5 text-xs text-ink">{reloadProblemText(name, p)}</p>;
}
