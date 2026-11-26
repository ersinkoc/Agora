// Every action of the window as a named command (ADR-0068 decision 6, docs/ux/keybindings.md): a stable id, its name (a Fluent key),
// where it acts, the heading the command palette lists it under, and its default keys. This list is data: the handlers are given by
// the parts of the window that can run them, while they are on screen (registry.ts). Ids are never renamed or reused, since a person's
// keybindings file names them.
import type { PaletteGroup } from '../components/CommandPalette';
import type { IconName } from '../components/Icon';
import { TOOL_PANELS } from '../layout/panels';
import { PRESETS } from '../layout/engine';
import { type Binding, type Stroke, parseBinding, strokeText } from './strokes';

/**
 * Where a command acts, which is also when it is offered:
 * - `anywhere`: on every page;
 * - `window`: the window with its panes (not a phone, not Settings);
 * - `pane`: the pane in focus;
 * - `tabs`: a tab of the pane group in focus;
 * - `side`: the side panel's rail.
 */
export type CommandContext = 'anywhere' | 'window' | 'pane' | 'tabs' | 'side';

export interface CommandDef {
  id: string;
  /** Its name, a Fluent key, with its arguments. */
  label: string;
  args?: Record<string, string | number>;
  group: PaletteGroup;
  icon?: IconName;
  context: CommandContext;
  /** The keys it has when the person changed nothing; several are alternatives (Ctrl+K and ⌘K). */
  keys: readonly string[];
  /** Fixed: its keys are not the person's to change, and no other command may have them (ADR-0114 safe mode). */
  fixed?: boolean;
  /** Not listed in the command palette, with the reason: `palette.open` is the palette, already open. */
  notInPalette?: boolean;
}

// The tools' icons, as the palette showed them before this rung.
const TOOL_ICON: Record<(typeof TOOL_PANELS)[number], IconName> = { terminal: 'terminal', changes: 'diff', files: 'files', manager: 'folder' };
const n9 = Array.from({ length: 9 }, (_, i) => i + 1);
const n5 = n9.slice(0, 5);

export const COMMANDS: readonly CommandDef[] = [
  { id: 'palette.open', label: 'palette-title', group: 'actions', icon: 'search', context: 'anywhere', keys: ['ctrl+k', 'meta+k', 'ctrl+shift+k'], notInPalette: true },
  { id: 'attention.next', label: 'palette-next-attention', group: 'go', icon: 'bell', context: 'anywhere', keys: ['n'] },
  { id: 'go.home', label: 'palette-go-home', group: 'go', icon: 'grid', context: 'anywhere', keys: [] },
  { id: 'agent.start', label: 'action-start-agent', group: 'actions', icon: 'plus', context: 'anywhere', keys: [] },
  ...TOOL_PANELS.map((p): CommandDef => ({ id: `tool.${p}`, label: `palette-open-${p}`, group: 'actions', icon: TOOL_ICON[p], context: 'window', keys: [] })),
  { id: 'terminal.new', label: 'palette-new-terminal', group: 'actions', icon: 'terminal', context: 'window', keys: [] },
  { id: 'go.machines', label: 'palette-go-machines', group: 'go', icon: 'machine', context: 'anywhere', keys: [] },
  { id: 'machines.allow', label: 'pair-window-allow', group: 'actions', icon: 'plus', context: 'anywhere', keys: [] },
  { id: 'window.lock', label: 'lock-now', group: 'actions', icon: 'lock', context: 'anywhere', keys: [] },
  { id: 'go.settings', label: 'palette-settings', group: 'go', icon: 'settings', context: 'anywhere', keys: [] },
  { id: 'go.keyboard', label: 'palette-keyboard', group: 'go', icon: 'keyboard', context: 'anywhere', keys: [] },
  { id: 'go.audit', label: 'audit-link', group: 'go', icon: 'history', context: 'anywhere', keys: [] },
  ...(['system', 'en', 'tr'] as const).map((l): CommandDef => ({ id: `language.${l}`, label: 'palette-language', args: { name: `language-${l}` }, group: 'appearance', icon: 'globe', context: 'anywhere', keys: [] })),
  { id: 'theme.editor', label: 'palette-theme-editor', group: 'appearance', icon: 'pencil', context: 'anywhere', keys: [] },
  { id: 'plugins.open', label: 'palette-plugins', group: 'appearance', icon: 'puzzle', context: 'anywhere', keys: [] },
  // The safe-mode key (ADR-0114): heard by the window before anything else (plugins/safe.ts `installSafeKey`), never through this list,
  // and no keybinding can move it or take it (FIXED_KEYS).
  { id: 'window.safe-mode', label: 'palette-safe-mode', group: 'appearance', icon: 'puzzle', context: 'anywhere', keys: ['ctrl+alt+shift+s', 'alt+shift+meta+s'], fixed: true },
  { id: 'window.agents', label: 'keys-cmd-agents', group: 'actions', icon: 'panelLeft', context: 'window', keys: ['ctrl+b', 'meta+b'] },
  ...n9.map((i): CommandDef => ({ id: `pane.tab-${i}`, label: 'keys-cmd-tab', args: { n: i }, group: 'go', icon: 'single', context: 'tabs', keys: [`alt+${i}`] })),
  ...n5.map((i): CommandDef => ({ id: `side.place-${i}`, label: 'keys-cmd-side', args: { n: i }, group: 'go', icon: 'panelRight', context: 'side', keys: [`alt+shift+${i}`] })),
  ...PRESETS.map((p): CommandDef => ({ id: `layout.preset-${p}`, label: 'keys-cmd-preset', args: { name: `layout-preset-${p}` }, group: 'actions', icon: 'layout', context: 'window', keys: [] })),
  { id: 'layout.maximize', label: 'keys-cmd-maximize', group: 'actions', icon: 'maximize', context: 'pane', keys: [] },
  { id: 'layout.split-right', label: 'layout-split-right', group: 'actions', icon: 'splitRight', context: 'pane', keys: [] },
  { id: 'layout.split-down', label: 'layout-split-down', group: 'actions', icon: 'splitDown', context: 'pane', keys: [] },
  { id: 'layout.close-pane', label: 'layout-close-pane', group: 'actions', icon: 'close', context: 'pane', keys: [] },
  { id: 'layout.reset', label: 'layout-reset', group: 'actions', icon: 'layout', context: 'window', keys: [] },
];

export const commandById = (id: string): CommandDef | undefined => COMMANDS.find((c) => c.id === id);

// A command's arguments that name a message (`language-tr`, `layout-preset-2x2`) are that message's words.
export const MESSAGE_ARGS = new Set(['name']);

// ---- What no keybinding may have --------------------------------------------------------------------------------------------------

/** Why a key is fixed: the reason is shown beside it in Settings > Keyboard and when a file asks for it. */
export type FixedReason = 'safe-mode' | 'navigation' | 'terminal' | 'editing';

/**
 * Keys no binding may use, as a stroke of any chord. Fixed because something the person relies on needs them:
 * - `safe-mode`: the safe-mode key (ADR-0114, docs/ux/plugins.md), heard by the window before anything else, with Ctrl or ⌘;
 * - `navigation`: moving through the window by keyboard (Tab, Shift+Tab, Esc, Enter, Space and the arrows; ADR-0024's keyboard rules);
 * - `terminal`: a terminal's copy, find and paste (terminal-fidelity.md), so a terminal always has them;
 * - `editing`: cut, copy, paste, select all, undo and redo in a text field.
 */
export const FIXED_KEYS: readonly { keys: string; reason: FixedReason }[] = [
  { keys: 'ctrl+alt+shift+s', reason: 'safe-mode' }, { keys: 'alt+shift+meta+s', reason: 'safe-mode' },
  ...['tab', 'shift+tab', 'escape', 'enter', 'space', 'up', 'down', 'left', 'right'].map((keys) => ({ keys, reason: 'navigation' as const })),
  ...['ctrl+shift+c', 'ctrl+insert', 'ctrl+shift+f', 'ctrl+shift+v', 'shift+insert', 'meta+c', 'meta+f', 'meta+v'].map((keys) => ({ keys, reason: 'terminal' as const })),
  ...['ctrl+a', 'ctrl+c', 'ctrl+v', 'ctrl+x', 'ctrl+z', 'ctrl+y', 'ctrl+shift+z', 'meta+a', 'meta+x', 'meta+z', 'shift+meta+z'].map((keys) => ({ keys, reason: 'editing' as const })),
];
const FIXED = new Map<string, FixedReason>(FIXED_KEYS.map((f) => [strokeText((parseBinding(f.keys) as Binding)[0] as Stroke), f.reason]));
/** Why a stroke is fixed, or undefined when a binding may use it. */
export const fixedReason = (s: Stroke): FixedReason | undefined => FIXED.get(strokeText(s));

/**
 * Commands no keybinding may name: answering a request (Allow, Deny, Allow always) is done by hand on its card, after the card has been
 * on screen long enough to read (T44), and never from a key bound in a file or from the palette (ADR-0020, ADR-0024). A file that names
 * one of these is refused, so a shared keybindings file cannot pose as one that answers for you.
 */
export const NEVER_COMMAND = /^(request|approval|approve|deny|allow)(\.|$)/;
