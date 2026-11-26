// The plugin bridge's side of the panel registry (ADR-0114 build step 2; layout.md, "Host-set facts"). Each panel a loaded, granted plugin
// declares is registered after the built-ins with facts the window sets itself: `kind` tool or global (as the manifest asks, nothing
// else), never `lone`, never `streams`, and its name as plain text from the manifest in `plugin.name` (the Fluent `label` is the
// window's word "Plugin"). An id is registered once, so a plugin can never stand in for a built-in panel; a pane naming a plugin panel
// this window does not have (a plugin removed, safe mode) shows the "unknown panel" placeholder.
//
// The drawing is the window's: it turns the pane's context into the plugin view at once (projection.ts `viewOf`) and gives the frame
// only that. A tool plugin bound to no agent shows the window's own folder chooser, outside the frame.
import { ToolChooser } from '../components/ToolPanes';
import { type PanelDef, type PanelInfo, type PanelRegistry, panelRegistry } from '../layout/panels';
import type { LoadedPlugin } from './store';
import { viewOf } from './projection';
import { PluginFrame } from './PluginFrame';

/** A plugin panel's id in a layout: `plugin.<plugin id>.<panel id>`, so it can never be a built-in's. */
export const pluginPanelId = (plugin: string, panel: string) => `plugin.${plugin}.${panel}`;
export const PLUGIN_LABEL = 'plugin-panel';

export function pluginPanels(plugins: readonly LoadedPlugin[]): PanelDef[] {
  const out: PanelDef[] = [];
  for (const p of plugins) {
    if (p.state !== 'ready') continue;
    const m = p.plugin.manifest;
    for (const decl of m.panels) {
      const id = pluginPanelId(m.id, decl.id);
      const name = decl.name ? `${m.name}: ${decl.name}` : m.name;
      const kind = decl.kind === 'tool' ? 'tool' : 'global';
      out.push({
        id, label: PLUGIN_LABEL, kind, binds: kind === 'tool' ? 'agent' : 'nothing', streams: false, lone: false, plugin: { name, dev: p.dev },
        render: (ctx) => {
          if (kind === 'tool' && !ctx.agent) return <ToolChooser panel="files" fleet={ctx.state.fleet} onPick={ctx.actions.rebind} />;
          const follows = ctx.bind && 'follow' in ctx.bind ? ctx.bind.follow : undefined;
          const view = viewOf({
            pane: ctx.pane, kind, ...(follows ? { follows } : {}), agent: ctx.agent, fleet: ctx.state.fleet, boards: ctx.state.boards, now: ctx.state.now,
            chatOf: ctx.sources.chatOf, loadDiff: ctx.sources.loadDiff,
          });
          // A new grant, or another bound agent (its conversation and changes are read from the agent the frame started with), starts
          // the frame again.
          return <PluginFrame key={`${id}:${p.granted.join(',')}:${ctx.agent?.id ?? ''}`} plugin={p.plugin} panel={decl} granted={p.granted} view={view} dev={p.dev} />;
        },
      });
    }
  }
  return out;
}

/** The window's registry with the plugins' panels after the built-ins (the first registration of an id wins). */
export function withPlugins(builtin: PanelRegistry, plugins: readonly LoadedPlugin[]): PanelRegistry {
  if (plugins.length === 0) return builtin;
  return panelRegistry<PanelDef>([...builtin.ids.map((id) => builtin.get(id) as PanelDef), ...pluginPanels(plugins)]);
}

/** Whether a registered panel is a plugin's (the window draws the marker beside its name). */
export const isPluginPanel = (def: PanelInfo | undefined): boolean => !!def?.plugin;
