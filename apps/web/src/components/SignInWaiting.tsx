import { t } from '../i18n';
import { formatPairNumber, type SignInApprovalModel } from './pairChoice';

// ADR-0065 decision 6: a browser on another machine signs in by the owner's approval instead of a code. While it waits it shows
// the number the owner's card shows; denied or out of time, it can ask again (a new request, a new number) or fall back to a code.
export function SignInWaiting({ approval, onAskAgain, onUseCode }: {
  approval: SignInApprovalModel;
  onAskAgain: () => void;
  onUseCode: () => void;
}) {
  const waiting = approval.state === 'waiting';
  return (
    <section className="grid max-w-[28rem] gap-3" aria-live="polite">
      {waiting ? (
        <>
          <p className="m-0">{t('signin-waiting')}</p>
          <div className="grid gap-1">
            <span className="text-xs text-muted">{t('pair-number')}</span>
            <span className="font-mono text-[length:var(--text-28)] tracking-wide text-ink">{formatPairNumber(approval.number)}</span>
          </div>
          <p className="m-0 text-xs text-muted">{t('signin-waiting-hint')}</p>
        </>
      ) : (
        <p role="alert" className="m-0 text-danger">{t(approval.state === 'denied' ? 'signin-denied' : 'signin-expired')}</p>
      )}
      <div className="flex flex-wrap gap-2">
        {!waiting && <button type="button" className="button button--primary" onClick={onAskAgain}>{t('signin-ask-again')}</button>}
        <button type="button" className="button" onClick={onUseCode}>{t('signin-use-code')}</button>
      </div>
    </section>
  );
}
