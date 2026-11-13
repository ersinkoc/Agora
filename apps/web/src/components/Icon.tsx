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
} as const;
export type IconName = keyof typeof PATHS;

export function Icon({ name, className = 'size-4' }: { name: IconName; className?: string }) {
  return <svg viewBox="0 0 16 16" className={`${className} flex-none rtl:[&_path]:origin-center`} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={PATHS[name]} /></svg>;
}
