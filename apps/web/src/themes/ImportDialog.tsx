// "Import from another app…" (docs/ux/themes.md): what a colour file of another app became, before anything is added. It shows the
// terminal's colours, what changed on the way (the converter's notes), and what the theme check says of the result: a theme it refuses
// cannot be added, one it warns about can. Adding goes through the store's own import, the same door as a theme file.
import * as Dialog from '@radix-ui/react-dialog';
import { useId } from 'react';
import { t } from '../i18n';
import { Icon } from '../components/Icon';
import { Button } from '../components/kit/Button';
import { type ReadTheme, readTheme, serialise, terminalColour } from './file';
import type { Converted } from './import/convert';
import { ANSI_NAMES, noExtras, type TerminalColours } from './look';
import { importErrorText, importFormatLabel, importNoteText, issueText, terminalLabel } from './messages';
import { type Saved, themeStore } from './store';
import { TerminalSample } from './TerminalSample';
import { type Look, type Tokens, baseTokens, looksOf } from './tokens';

export function ImportDialog({ result, onClose, onAdded }: { result: Converted; onClose: () => void; onAdded: (r: Saved) => void }) {
  const id = useId();
  const judged = result.ok ? readTheme(result.file) : undefined;
  const look = previewLook(result, judged);
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="kit-overlay" />
        <Dialog.Content className="kit-dialog w-[min(34rem,calc(100vw-2rem))]" aria-describedby={id} data-theme-import>
          <span aria-hidden className="kit-chip kit-tone-accent"><Icon name="palette" /></span>
          <Dialog.Title className="kit-dialog__title">{t('theme-import-title')}</Dialog.Title>
          <div id={id} className="kit-dialog__body grid min-w-0 gap-3 text-sm">
            {!result.ok ? <p role="alert" className="m-0 text-ink">{importErrorText(result.error)}</p> : <>
              <p className="m-0 text-ink">{t('theme-import-from', { format: importFormatLabel(result.format), name: result.name })}</p>
              {look && <div className="grid gap-2" aria-label={t('theme-import-palette')} role="group">
                <TerminalSample look={look} scheme={result.scheme} />
                <div className="grid grid-cols-8 gap-1" data-import-palette>
                  {ANSI_NAMES.map((n) => <span key={n} title={terminalLabel(n)} className="h-4 rounded-xs border border-line"
                    style={{ background: terminalColour(look, result.scheme, n) }} />)}
                </div>
              </div>}
              {judged && !judged.ok && <div className="grid gap-1" role="alert">
                <p className="m-0 text-danger">{t('theme-import-refused')}</p>
                <ul className="m-0 grid list-none gap-1 p-0 text-xs text-ink">{judged.issues.map((i) => <li key={JSON.stringify(i)}>{issueText(i)}</li>)}</ul>
              </div>}
              <div className="grid gap-1">
                <p className="m-0 text-xs font-semibold text-ink">{t('theme-import-notes')}</p>
                <ul className="m-0 grid list-disc gap-1 ps-4 text-xs" data-import-notes>{result.notes.map((n) => <li key={JSON.stringify(n)}>{importNoteText(n)}</li>)}</ul>
              </div>
              {judged?.ok && judged.warnings && <div className="grid gap-1">
                <p className="m-0 text-xs font-semibold text-ink">{t('theme-import-warnings')}</p>
                <ul className="m-0 grid list-none gap-1 p-0 text-xs">{judged.warnings.map((w) => <li key={JSON.stringify(w)}>{issueText(w)}</li>)}</ul>
              </div>}
            </>}
          </div>
          <div className="kit-dialog__actions">
            <Button onClick={onClose}>{t('theme-import-cancel')}</Button>
            {result.ok && <Button variant="primary" disabled={!judged?.ok} onClick={() => onAdded(themeStore.import(serialise(result.file)))}>{t('theme-import-add')}</Button>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

// The look to preview: the accepted theme's, or (refused) the file's colours over the scheme's defaults, so the person sees why.
function previewLook(result: Converted, judged: ReadTheme | undefined): Look | undefined {
  if (!result.ok) return undefined;
  if (judged?.ok) return looksOf(judged.theme)[result.scheme];
  const section = result.file[result.scheme] as { tokens?: Partial<Tokens>; terminal?: TerminalColours };
  return { ...noExtras(), tokens: { ...baseTokens(result.scheme), ...section.tokens }, terminal: { ...section.terminal } };
}
