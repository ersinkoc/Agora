import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import { SignInPage } from '../screens';
import { Console } from './Console';
import { LockScreen, ScreenPasswordSettings } from './Locks';
import { demoSource } from '../terminal/source';

describe('screen password in the window', () => {
  beforeEach(() => setLanguage('en'));

  it('asks for the password only when the hub says this browser needs it', () => {
    const without = renderToStaticMarkup(<SignInPage onSubmit={() => {}} />);
    expect(without).not.toContain('type="password"');
    const required = renderToStaticMarkup(<SignInPage onSubmit={() => {}} passwordRequired />);
    expect(required).toContain('type="password"');
    expect(required).toContain('Screen password');
    expect(required).toContain('autoComplete="current-password"');
  });

  it('has a Lock button in the title bar when signed in, and none without a session', () => {
    const props = { agents: [], machines: [], approvals: [], messages: [], terminalFor: () => demoSource('x'), onOpen: () => {}, onResolve: () => {}, link: 'live' as const };
    expect(renderToStaticMarkup(<Console {...props} onLock={() => {}} />)).toContain('>Lock<');
    expect(renderToStaticMarkup(<Console {...props} />)).not.toContain('>Lock<');
  });

  it('shows a lock screen with a password field and a way out', () => {
    const html = renderToStaticMarkup(<LockScreen onUnlock={async () => ({ ok: true })} onSignOut={() => {}} />);
    expect(html).toContain('type="password"');
    expect(html).toContain('Unlock');
    expect(html).toContain('Sign out instead');
  });

  it('asks for the current password only when one is set, and offers the idle lock off by default', () => {
    const set = async () => ({ ok: true as const });
    const first = renderToStaticMarkup(<ScreenPasswordSettings passwordSet={false} onSet={set} idle={0} onIdle={() => {}} />);
    expect(first).not.toContain('name="current"');
    expect(first).toContain('Set password');
    expect(first).toMatch(/<option value="0" selected="">Never<\/option>/);
    const change = renderToStaticMarkup(<ScreenPasswordSettings passwordSet onSet={set} idle={10} onIdle={() => {}} />);
    expect(change).toContain('name="current"');
    expect(change).toMatch(/<option value="10" selected="">After 10 minutes<\/option>/);
  });
});
