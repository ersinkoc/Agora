import * as Dialog from '@radix-ui/react-dialog';
import { type CSSProperties, useId, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { themeName } from '../themes/messages';
import { type Mode, themeStore } from '../themes/store';
import { type Look, type Scheme, type Theme, hasScheme, looksOf } from '../themes/tokens';
import { Icon } from './Icon';
import { Button } from './kit/Button';
import { SegmentedControl } from './kit/SegmentedControl';

const MODES: { value: Mode; label: string }[] = [
  { value: 'system', label: 'theme-system-short' },
  { value: 'dark', label: 'theme-dark' },
  { value: 'light', label: 'theme-light' },
];

// The title bar's one theme control opens the same visual chooser in the web window and the desktop shell. Every card is a small,
// inert drawing made only from the already-validated theme tokens; choosing one still goes through the theme store's normal path.
export function ThemePicker() {
  const hint = useId();
  const state = useSyncExternalStore(themeStore.subscribe, themeStore.state, themeStore.state);
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Scheme>(() => themeStore.scheme());
  const shown = themeStore.byId(state.choice[tab]);
  const setMode = (mode: string) => {
    const next = mode as Mode;
    themeStore.setMode(next);
    if (next !== 'system') setTab(next);
  };

  return (
    <Dialog.Root open={open} onOpenChange={(next) => { setOpen(next); if (next) setTab(themeStore.scheme()); }}>
      <Dialog.Trigger asChild>
        <button type="button" className="ide-iconbtn size-7" aria-label={t('theme-chooser-open')} title={t('theme-chooser-open')}>
          <Icon name="palette" className="size-4" />
        </button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="kit-overlay" />
        <Dialog.Content className="kit-dialog max-h-[min(48rem,calc(100vh-2rem))] w-[min(52rem,calc(100vw-2rem))]" aria-describedby={hint} data-theme-chooser>
          <span aria-hidden className="kit-chip kit-tone-accent"><Icon name="palette" /></span>
          <Dialog.Title className="kit-dialog__title">{t('theme-chooser-title')}</Dialog.Title>
          <div className="kit-dialog__body grid min-w-0 gap-4 text-sm">
            <p id={hint} className="m-0">{t('theme-chooser-hint')}</p>
            <div className="grid gap-3 sm:grid-cols-2">
              <SegmentedControl label={t('theme-label')} value={state.mode} onChange={setMode}
                items={MODES.map((mode) => ({ value: mode.value, label: t(mode.label) }))} />
              <SegmentedControl label={t('theme-chooser-scheme')} value={tab} onChange={(value) => setTab(value as Scheme)}
                items={(['dark', 'light'] as const).map((scheme) => ({ value: scheme, label: t(`theme-${scheme}`) }))} />
            </div>
            <div className="flex min-w-0 items-center justify-between gap-2">
              <span className="font-semibold text-ink">{t(`theme-for-${tab}`)}</span>
              {shown && <span className="truncate text-xs">{t('theme-chooser-current', { name: themeName(shown) })}</span>}
            </div>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-3" role="group" aria-label={t(`theme-for-${tab}`)}>
              {themeStore.themes().filter((theme) => hasScheme(theme, tab)).map((theme) => (
                <ThemeChoice key={theme.id} theme={theme} scheme={tab} selected={state.choice[tab] === theme.id}
                  onChoose={() => themeStore.choose(theme.id)} />
              ))}
            </div>
          </div>
          <div className="kit-dialog__actions">
            <Dialog.Close asChild><Button variant="primary">{t('theme-chooser-done')}</Button></Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function ThemeChoice({ theme, scheme, selected, onChoose }: { theme: Theme; scheme: Scheme; selected: boolean; onChoose: () => void }) {
  const look = looksOf(theme)[scheme] as Look;
  const tokens = look.tokens;
  const style = { background: tokens['surface-0'], color: tokens.text, borderColor: selected ? tokens.accent : tokens.border } as CSSProperties;
  return (
    <button type="button" aria-pressed={selected} aria-label={themeName(theme)} onClick={onChoose}
      className="group grid min-w-0 gap-2 rounded-m border-2 p-2 text-start transition-[border-color,transform] hover:-translate-y-0.5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2"
      style={style} data-theme-choice={theme.id}>
      <ThemeMiniature look={look} />
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="truncate text-xs font-semibold">{themeName(theme)}</span>
        {selected && <span className="ms-auto grid size-4 flex-none place-items-center rounded-full" style={{ background: tokens.accent, color: tokens['on-accent'] }}
          title={t('theme-chooser-selected')}><Icon name="check" className="size-3" /></span>}
      </span>
    </button>
  );
}

function ThemeMiniature({ look }: { look: Look }) {
  const c = look.tokens;
  return (
    <span aria-hidden className="grid h-20 grid-cols-[2.4rem_1fr] overflow-hidden rounded-s border" style={{ borderColor: c.border, background: c['surface-0'] }}>
      <span className="grid content-start gap-1 border-e p-1.5" style={{ borderColor: c.border, background: c['surface-1'] }}>
        <span className="h-1.5 w-4 rounded-full" style={{ background: c.accent }} />
        <span className="mt-1 h-1 w-full rounded-full" style={{ background: c['text-muted'] }} />
        <span className="h-1 w-3/4 rounded-full" style={{ background: c['text-muted'] }} />
        <span className="h-1 w-5/6 rounded-full" style={{ background: c['text-muted'] }} />
      </span>
      <span className="grid grid-rows-[1rem_1fr_0.8rem]">
        <span className="border-b px-1.5" style={{ borderColor: c.border, background: c['surface-1'] }}>
          <span className="block h-1.5 w-8 translate-y-1 rounded-full" style={{ background: c.text }} />
        </span>
        <span className="grid content-center gap-1.5 p-2">
          <span className="h-4 rounded-xs border" style={{ borderColor: c.border, background: c['surface-2'] }} />
          <span className="flex gap-1">
            <span className="h-2 flex-1 rounded-full" style={{ background: c['state-working'] }} />
            <span className="h-2 flex-1 rounded-full" style={{ background: c['state-finished'] }} />
            <span className="h-2 flex-1 rounded-full" style={{ background: c['state-needs-you'] }} />
          </span>
        </span>
        <span className="border-t" style={{ borderColor: c.border, background: c['surface-1'] }} />
      </span>
    </span>
  );
}
