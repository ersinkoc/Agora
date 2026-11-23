import { t } from '../i18n';
import { Icon } from './Icon';
import { Button, Callout } from './kit';
import { formatPairNumber, type SignInApprovalModel } from './pairChoice';

// ADR-0065 decision 6: a browser on another machine signs in by the owner's approval instead of a code. While it waits it shows
// the number the owner's card shows; denied or out of time, it can ask again (a new request, a new number) or fall back to a code.
// Inside the sign-in card: the number in an accent-tinted box; a refusal is a danger callout, running out of time a neutral one.
export function SignInWaiting({ approval, onAskAgain, onUseCode }: {
  approval: SignInApprovalModel;
  onAskAgain: () => void;
  onUseCode: () => void;
}) {
  const waiting = approval.state === 'waiting';
  const denied = approval.state === 'denied';
  return (
    <section className="signin-wait" aria-live="polite">
      {waiting ? (
        <>
          <p className="signin-wait__status"><Icon name="clock" className="size-4 text-muted" />{t('signin-waiting')}</p>
          <div className="signin-wait__number">
            <span className="signin-wait__label">{t('pair-number')}</span>
            <span className="signin-wait__digits">{formatPairNumber(approval.number)}</span>
          </div>
          <p className="signin__hint">{t('signin-waiting-hint')}</p>
        </>
      ) : (
        <Callout tone={denied ? 'danger' : 'info'} icon={denied ? 'close' : 'clock'} title={t(denied ? 'signin-denied' : 'signin-expired')} alert />
      )}
      <div className="signin-wait__actions">
        {!waiting && <Button variant="primary" onClick={onAskAgain}>{t('signin-ask-again')}</Button>}
        <Button onClick={onUseCode}>{t('signin-use-code')}</Button>
      </div>
    </section>
  );
}
