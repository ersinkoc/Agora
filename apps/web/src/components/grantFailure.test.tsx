// @vitest-environment jsdom
// The maintainer's window (2026-11-27): a browser at a new address has a new device key that the machine does not trust yet, and Allow
// said only "did not go through. Try again", which never helps. A refused signature now says what to do.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { grantFailureKey } from '../net/useGrantFlow';
import { GrantDialog } from './GrantDialog';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('a refused grant', () => {
  it('names an untrusted device, and is the plain failure otherwise', () => {
    expect(grantFailureKey({ code: 'approval.signature_invalid', params: {} })).toBe('approval-not-trusted');
    expect(grantFailureKey({ code: 'node.offline' })).toBe('grant-failed');
    expect(grantFailureKey(new Error('x'))).toBe('grant-failed');
    expect(grantFailureKey(undefined)).toBe('grant-failed');
  });

  it('tells the person to trust this device on the machine', () => {
    act(() => root.render(<GrantDialog machine="blackview" agent="claude-code · mint" kind="typing" failed reason={grantFailureKey({ code: 'approval.signature_invalid' })} onAllow={() => {}} onClose={() => {}} />));
    const alert = document.querySelector('[role=alert]')!;
    expect(alert.textContent).toContain('This device is not trusted by that machine yet');
    expect(alert.textContent).not.toContain('Try again');
  });
});
