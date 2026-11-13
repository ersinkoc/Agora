// The screen password in the window (ADR-0067): the lock screen a window shows after it sat idle, and the This device section that
// sets the password and the idle lock. Locking hides the window only; agents keep running on their machines.
import { useState } from 'react';
import { t } from '../i18n';
import { LanguagePicker } from './LanguagePicker';
import { IDLE_CHOICES, type IdleMinutes } from '../net/idle';
import { MIN_PASSWORD, type PasswordResult } from '../net/session';

const notice = (r: PasswordResult) => (r.ok ? undefined : `password-${r.reason}`);

export function LockScreen({ onUnlock, onSignOut }: { onUnlock: (password: string) => Promise<PasswordResult>; onSignOut: () => void }) {
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | undefined>();
  return (
    <main className="page">
      <div className="flex justify-end"><LanguagePicker /></div>
      <h1>{t('lock-title')}</h1>
      <p>{t('lock-intro')}</p>
      <form className="stack" onSubmit={(e) => {
        e.preventDefault();
        if (!password || busy) return;
        setBusy(true);
        void onUnlock(password).then((r) => { setBusy(false); setProblem(notice(r)); if (!r.ok) setPassword(''); });
      }}>
        <label>
          <span className="muted">{t('signin-password')}</span>{' '}
          <input type="password" name="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" autoFocus />
        </label>
        <p><button type="submit" className="button button--primary" disabled={busy}>{t('lock-unlock')}</button></p>
      </form>
      <p><button type="button" className="button" onClick={onSignOut}>{t('lock-sign-out')}</button></p>
      {problem && <p role="alert">{t(problem)}</p>}
    </main>
  );
}

// Set or change the screen password, and choose the idle lock. `passwordSet`: one is set (changing it needs it).
export function ScreenPasswordSettings({ passwordSet, onSet, idle, onIdle }: {
  passwordSet: boolean;
  onSet: (password: string, current?: string) => Promise<PasswordResult>;
  idle: IdleMinutes;
  onIdle: (minutes: IdleMinutes) => void;
}) {
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [again, setAgain] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | undefined>();
  const submit = () => {
    if (next.length < MIN_PASSWORD) { setResult('password-too-short'); return; }
    if (next !== again) { setResult('password-mismatch'); return; }
    setBusy(true);
    void onSet(next, passwordSet ? current : undefined).then((r) => {
      setBusy(false);
      setResult(r.ok ? 'password-saved' : notice(r));
      if (r.ok) { setCurrent(''); setNext(''); setAgain(''); }
    });
  };
  return (
    <section className="stack" aria-labelledby="screen-password">
      <h2 id="screen-password">{t('password-title')}</h2>
      <p>{t(passwordSet ? 'password-intro-set' : 'password-intro')}</p>
      <form className="stack" onSubmit={(e) => { e.preventDefault(); if (!busy) submit(); }}>
        {passwordSet && (
          <label>
            <span className="muted">{t('password-current')}</span>{' '}
            <input type="password" name="current" value={current} onChange={(e) => setCurrent(e.target.value)} autoComplete="current-password" />
          </label>
        )}
        <label>
          <span className="muted">{t('password-new')}</span>{' '}
          <input type="password" name="new" value={next} onChange={(e) => setNext(e.target.value)} autoComplete="new-password" minLength={MIN_PASSWORD} />
        </label>
        <label>
          <span className="muted">{t('password-again')}</span>{' '}
          <input type="password" name="again" value={again} onChange={(e) => setAgain(e.target.value)} autoComplete="new-password" />
        </label>
        <p><button type="submit" className="button" disabled={busy}>{t(passwordSet ? 'password-change' : 'password-set')}</button></p>
        {result && <p role={result === 'password-saved' ? 'status' : 'alert'}>{t(result, { min: MIN_PASSWORD })}</p>}
      </form>
      <label>
        <span className="muted">{t('idle-lock')}</span>{' '}
        <select value={idle} onChange={(e) => onIdle(Number(e.target.value) as IdleMinutes)}>
          {IDLE_CHOICES.map((m) => <option key={m} value={m}>{m === 0 ? t('idle-lock-off') : t('idle-lock-minutes', { minutes: m })}</option>)}
        </select>
      </label>
      <p className="muted">{t(passwordSet ? 'idle-lock-hint' : 'idle-lock-hint-no-password')}</p>
    </section>
  );
}
