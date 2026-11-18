import { t } from '../i18n';
import { Icon } from '../components/Icon';

// What floats over the bottom of a terminal, in one column so nothing can lie on another: a clipboard offer, "Jump to latest", and the
// address of the link under the pointer (last, nearest the edge). It never reaches into the text above: the column is only as tall as
// what it holds.
export function TerminalFloat({ atBottom, onJump, offer, onCopy, onIgnore, link }: {
  atBottom: boolean;
  onJump: () => void;
  offer?: string;
  onCopy: () => void;
  onIgnore: () => void;
  link?: string;
}) {
  return (
    <div className="terminal-float">
      {!atBottom && (
        <button type="button" className="terminal-jump" onClick={onJump}>
          <Icon name="chevron" className="size-4 rotate-90" />{t('terminal-jump-bottom')}
        </button>
      )}
      {offer !== undefined && (
        <div role="status" className="terminal-offer">
          <p className="terminal-offer__text">{t('terminal-copy-offer')} <span className="text-muted">{t('terminal-copy-offer-length', { count: offer.length })}</span></p>
          <pre className="terminal-offer__preview" dir="ltr" tabIndex={0}>{offer}</pre>
          <div className="flex gap-2">
            <button type="button" className="button" onClick={onCopy}>{t('terminal-copy-offer-copy')}</button>
            <button type="button" className="button" onClick={onIgnore}>{t('terminal-copy-offer-ignore')}</button>
          </div>
        </div>
      )}
      {link !== undefined && <p className="terminal-link" dir="ltr">{link}</p>}
    </div>
  );
}
