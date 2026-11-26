// A few lines of a terminal, drawn in a theme's own palette: in the editor's preview and in the catalog's (ThemeCatalog.tsx).
import { t } from '../i18n';
import { terminalColour } from './file';
import type { TerminalKey } from './look';
import type { Look, Scheme } from './tokens';

// A few lines of a terminal in the theme's palette (its colours were checked when the theme was read: #rrggbb only).
export function TerminalSample({ look, scheme }: { look: Look; scheme: Scheme }) {
  const c = (k: TerminalKey) => terminalColour(look, scheme, k);
  return (
    <pre className="m-0 overflow-hidden rounded-s p-2 font-mono text-xs leading-5" style={{ background: c('background'), color: c('foreground') }} data-terminal-sample>
      <span style={{ color: c('bright-black') }}>{t('theme-preview-sample-command')}</span>{'\n'}
      <span style={{ color: c('green') }}>{t('theme-preview-sample-pass')}</span>{'  '}<span style={{ color: c('red') }}>{t('theme-preview-sample-fail')}</span>{'\n'}
      <span style={{ color: c('yellow') }}>{t('theme-preview-sample-note')}</span>{' '}<span style={{ color: c('blue') }}>src/</span><span style={{ color: c('magenta') }}>lib</span><span style={{ color: c('cyan') }}>.rs</span>
      <span className="inline-block w-[0.6em]" style={{ background: c('cursor') }}>{' '}</span>
    </pre>
  );
}
