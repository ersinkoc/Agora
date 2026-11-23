// The screen password in the window (ADR-0067): the This device section that sets the password and the idle lock. Locking (Lock, or
// the idle lock) ends the window's session at the hub; agents keep running on their machines.
import { useId, useState } from 'react';
import { t } from '../i18n';
import { IDLE_CHOICES, type IdleMinutes } from '../net/idle';
import { MIN_PASSWORD, type PasswordResult } from '../net/session';
import { Icon } from './Icon';
import { Button, Pill, SettingRow } from './kit';
import { SettingsCard } from './SettingsCard';

const notice = (r: PasswordResult) => (r.ok ? undefined : `password-${r.reason}`);

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
  const id = useId();
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
  const field = (name: 'current' | 'new' | 'again', label: string, value: string, set: (v: string) => void, auto: string) => (
    <div className="set-form__row">
      <label htmlFor={`${id}-${name}`} className="set-form__label">{label}</label>
      <input id={`${id}-${name}`} type="password" name={name} value={value} onChange={(e) => set(e.target.value)} autoComplete={auto} minLength={name === 'new' ? MIN_PASSWORD : undefined} />
    </div>
  );
  return (
    <SettingsCard icon="lock" title={t('password-title')} description={t(passwordSet ? 'password-desc-set' : 'password-desc')}
      end={<Pill tone={passwordSet ? 'finished' : 'plain'} icon={passwordSet ? <Icon name="check" className="size-3" /> : undefined}>{t(passwordSet ? 'password-state-set' : 'password-state-unset')}</Pill>}>
      <form className="set-form" onSubmit={(e) => { e.preventDefault(); if (!busy) submit(); }}>
        {passwordSet && field('current', t('password-current'), current, setCurrent, 'current-password')}
        {field('new', t('password-new'), next, setNext, 'new-password')}
        {field('again', t('password-again'), again, setAgain, 'new-password')}
        <div className="set-form__row">
          <span aria-hidden className="set-form__label" />
          <div className="set-form__actions">
            <Button type="submit" variant="primary" disabled={busy}>{t(passwordSet ? 'password-change' : 'password-set')}</Button>
            {result && <p role={result === 'password-saved' ? 'status' : 'alert'} className={`m-0 ${result === 'password-saved' ? 'text-finished' : 'text-danger'}`}>{t(result, { min: MIN_PASSWORD })}</p>}
          </div>
        </div>
      </form>
      <SettingRow label={t('idle-lock')} hint={t(passwordSet ? 'idle-lock-hint' : 'idle-lock-hint-no-password')}>
        {(labelId, hintId) => (
          <select aria-labelledby={labelId} aria-describedby={hintId} value={idle} onChange={(e) => onIdle(Number(e.target.value) as IdleMinutes)}>
            {IDLE_CHOICES.map((m) => <option key={m} value={m}>{m === 0 ? t('idle-lock-off') : t('idle-lock-minutes', { minutes: m })}</option>)}
          </select>
        )}
      </SettingRow>
    </SettingsCard>
  );
}
