// Small line icons for the window chrome (16px grid, currentColor). Decorative: the control that holds one carries the label.
const PATHS = {
  plus: 'M8 3v10M3 8h10',
  close: 'M4 4l8 8M12 4l-8 8',
  chevron: 'M6 4l4 4-4 4',
  folder: 'M2 4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.4 1.5h5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z',
  terminal: 'M2.5 3.5h11a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1zM4.5 6.5l2 1.5-2 1.5M8 10h3',
  files: 'M4 2.5h5l3 3v8H4zM9 2.5v3h3',
  history: 'M2.5 8a5.5 5.5 0 1 0 1.8-4M2.5 3v3h3M8 5v3.2l2 1.3',
  maximize: 'M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3',
  single: 'M2.5 3.5h11v9h-11z',
  grid: 'M2.5 3.5h11v9h-11zM8 3.5v9',
  panelLeft: 'M2.5 3.5h11v9h-11zM6 3.5v9',
  panelRight: 'M2.5 3.5h11v9h-11zM10 3.5v9',
  search: 'M7 11.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9zM10.5 10.5L14 14',
  branch: 'M5 2.5v7M5 12.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM11 5.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM11 5.5c0 3-6 2-6 4',
  machine: 'M2.5 3.5h11v6h-11zM6 12.5h4M8 9.5v3',
  monitor: 'M2.5 3h11v7.5h-11zM6 13.5h4M8 10.5v3',
  moon: 'M13 9.5A5.5 5.5 0 0 1 6.5 3a5.5 5.5 0 1 0 6.5 6.5z',
  sun: 'M8 10.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1 1M11.6 11.6l1 1M3.4 12.6l1-1M11.6 4.4l1-1',
  copy: 'M5.5 5.5h7v8h-7zM3.5 10.5v-8h7',
  play: 'M5 3.5l7 4.5-7 4.5z',
  shieldOff: 'M8 2l5 2v4c0 3-2.2 5-5 6-2.8-1-5-3-5-6V4zM3 3l10 10',
  lock: 'M4 7.5h8v6H4zM5.5 7.5V5a2.5 2.5 0 0 1 5 0v2.5',
  // An agent that acts without asking (NoAskBadge): a lightning bolt, never the warning sign the quota chip uses.
  bolt: 'M9 1.5L3.5 9H8l-1 5.5L12.5 7H8z',
  // ADR-0068 layouts: split beside, split below, restore from maximized, the layout menu, more actions.
  splitRight: 'M2.5 3.5h11v9h-11zM8 3.5v9',
  splitDown: 'M2.5 3.5h11v9h-11zM2.5 8h11',
  restore: 'M6 3v3H3M10 3v3h3M6 13v-3H3M10 13v-3h3',
  layout: 'M2.5 3.5h11v9h-11zM7 3.5v9M7 8h6.5',
  more: 'M3.5 8h.5M7.75 8h.5M12 8h.5',
  // What an agent did in a conversation: edit, fetch, delete, move, think, and a step of any other kind.
  pencil: 'M10.5 3l2.5 2.5-7 7H3.5V10zM9 4.5l2.5 2.5',
  globe: 'M8 14a6 6 0 1 0 0-12 6 6 0 0 0 0 12zM2 8h12M8 2c-2 2.2-2 9.8 0 12M8 2c2 2.2 2 9.8 0 12',
  trash: 'M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9',
  move: 'M2.5 8h10M9.5 5l3 3-3 3',
  think: 'M6 12.5h4M6.5 14h3M8 2a4 4 0 0 0-2.3 7.3c.5.4.8.9.8 1.5V11h3v-.2c0-.6.3-1.1.8-1.5A4 4 0 0 0 8 2z',
  step: 'M8 10a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
  // The right panel's rail: what needs you, what changed, the shared board, messages, what was spent; plus a check, a down chevron, a clock.
  bell: 'M4 11V7a4 4 0 0 1 8 0v4l1 1.5H3zM6.5 13.5a1.5 1.5 0 0 0 3 0',
  diff: 'M5 2.5v7M5 12.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM11 13.5V6.5M11 3.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zM8 4.5h-1M7.5 4.5L9 3M7.5 4.5L9 6',
  board: 'M2.5 3h3.5v10H2.5zM6.5 3H10v6H6.5zM10.5 3H14v8h-3.5z',
  chat: 'M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2z',
  warn: 'M8 2.5l6 10.5H2zM8 6.5v3M8 11.2v.1',
  check: 'M3.5 8.5l3 3 6-7',
  // Usage (bars, never money), and the file manager: upload, download, a code file, a picture.
  gauge: 'M2.5 13.5h11M4.5 12V8M8 12V3.5M11.5 12V6.5',
  upload: 'M8 11V3M5 6l3-3 3 3M3 13h10',
  download: 'M8 3v8M5 8l3 3 3-3M3 13h10',
  code: 'M6 5L3 8l3 3M10 5l3 3-3 3',
  image: 'M2.5 3.5h11v9h-11zM2.5 11l3.5-3.5 3 3 2-2 2.5 2.5',
  chevronDown: 'M4 6l4 4 4-4',
  // The title bar's Settings (a gear) and Machines (stacked servers).
  settings: 'M6.9 3.2L7.1 1.7L8.9 1.7L9.1 3.2L10.6 3.8L11.9 2.9L13.1 4.1L12.2 5.4L12.8 6.9L14.3 7.1L14.3 8.9L12.8 9.1L12.2 10.6L13.1 11.9L11.9 13.1L10.6 12.2L9.1 12.8L8.9 14.3L7.1 14.3L6.9 12.8L5.4 12.2L4.1 13.1L2.9 11.9L3.8 10.6L3.2 9.1L1.7 8.9L1.7 7.1L3.2 6.9L3.8 5.4L2.9 4.1L4.1 2.9L5.4 3.8zM8 10.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4z',
  stack: 'M2.5 2.5h11v4h-11zM2.5 9.5h11v4h-11zM5 4.5h.01M5 11.5h.01M8 4.5h4M8 11.5h4',
  key: 'M5.3 12.3a2.3 2.3 0 1 0 0-4.6 2.3 2.3 0 0 0 0 4.6zM7 8.3l5.3-5.3M10.7 4.7l1.6 1.6M9.3 6l1.4 1.4',
  palette: 'M8 2.3a5.7 5.7 0 1 0 0 11.4c.8 0 1.2-.5 1.2-1.1 0-.9-.9-1.2-.9-2 0-.7.5-1.2 1.2-1.2h1.8A2.3 2.3 0 0 0 13.7 7c0-2.6-2.5-4.7-5.7-4.7zM5 7.7h.01M7 5h.01M10 5.3h.01',
  // Systems, for a machine or a browser: drawn as plain outlines in the one stroke, not as their makers' logos.
  windows: 'M2.7 3.7l4.6-.7v4.7H2.7zM8.7 2.8l4.6-.7v5.6H8.7zM2.7 8.3h4.6v4.7l-4.6-.7zM8.7 8.3h4.6v5.6l-4.6-.7z',
  apple: 'M10.33 2.67c-.67.07-1.47.53-1.87 1.07-.4.47-.67 1.2-.53 1.87.73 0 1.47-.47 1.87-1 .4-.53.67-1.2.53-1.93zM12.33 11c-.33.73-.53 1.07-.93 1.73-.6.87-1.4 1.93-2.4 1.93-.87 0-1.13-.6-2.33-.6s-1.47.6-2.33.6c-1 0-1.73-1-2.33-1.87C1.07 10.4.93 7.67 1.87 6.33c.67-.93 1.73-1.53 2.73-1.53 1 0 1.67.6 2.53.6.8 0 1.33-.6 2.53-.6.87 0 1.87.47 2.53 1.33-2.2 1.2-1.87 4.4.13 4.87z',
  linux: 'M8 2c-1.47 0-2.33 1.2-2.33 2.8 0 1.07-.4 1.73-1.07 2.67C3.73 8.67 3.33 9.73 3.33 10.87 3.33 12.6 5.33 14 8 14s4.67-1.4 4.67-3.13c0-1.13-.4-2.2-1.27-3.4-.67-.93-1.07-1.6-1.07-2.67C10.33 3.2 9.47 2 8 2zM6.8 5.47h.01M9.2 5.47h.01M7.2 6.8 8 7.33l.8-.53',
  phone: 'M5 1.5h6a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1zM7.3 12.5h1.4',
  clock: 'M8 14a6 6 0 1 0 0-12 6 6 0 0 0 0 12zM8 4.5V8l2.3 1.4',
  // A machine's system on My machines (the tile beside its name): a penguin, an apple, four panes. Generic outlines, not logos.
  osLinux: 'M8 2c-1.47 0 -2.33 1.2 -2.33 2.8 0 1.07 -0.4 1.73 -1.07 2.67C3.73 8.67 3.33 9.73 3.33 10.87 3.33 12.6 5.33 14 8 14s4.67 -1.4 4.67 -3.13c0 -1.13 -0.4 -2.2 -1.27 -3.4 -0.67 -0.93 -1.07 -1.6 -1.07 -2.67C10.33 3.2 9.47 2 8 2zM6.8 5.47h0.01M9.2 5.47h0.01M7.2 6.8 8 7.33l0.8 -0.53',
  osMac: 'M10.33 2.67c-0.67 0.07 -1.47 0.53 -1.87 1.07 -0.4 0.47 -0.67 1.2 -0.53 1.87 0.73 0 1.47 -0.47 1.87 -1 0.4 -0.53 0.67 -1.2 0.53 -1.93zM12.33 11c-0.33 0.73 -0.53 1.07 -0.93 1.73 -0.6 0.87 -1.4 1.93 -2.4 1.93 -0.87 0 -1.13 -0.6 -2.33 -0.6s-1.47 0.6 -2.33 0.6c-1 0 -1.73 -1 -2.33 -1.87C1.07 10.4 0.93 7.67 1.87 6.33c0.67 -0.93 1.73 -1.53 2.73 -1.53 1 0 1.67 0.6 2.53 0.6 0.8 0 1.33 -0.6 2.53 -0.6 0.87 0 1.87 0.47 2.53 1.33 -2.2 1.2 -1.87 4.4 0.13 4.87z',
  // A plugin's panel (ADR-0114): the marker the window draws beside its name.
  puzzle: 'M3 5.5h2.6a1.4 1.4 0 1 1 2.8 0H11v2.6a1.4 1.4 0 1 1 0 2.8v2.6H3z',
  osWindows: 'M2.67 3.67 7.33 3v4.67H2.67zM8.67 2.8l4.67 -0.67v5.53h-4.67zM2.67 8.33h4.67v4.67l-4.67 -0.67zM8.67 8.33h4.67v5.53l-4.67 -0.67z',
} as const;
export type IconName = keyof typeof PATHS;

export function Icon({ name, className = 'size-4' }: { name: IconName; className?: string }) {
  return <svg viewBox="0 0 16 16" className={`${className} flex-none rtl:[&_path]:origin-center`} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={PATHS[name]} /></svg>;
}
