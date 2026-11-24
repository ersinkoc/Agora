// @vitest-environment jsdom
// ADR-0059 decision 5: an agent may ask for a helper that acts without asking. The card names the option and what it means, offers no
// grant, and allows it only after the same confirmation a start from the window asks for, with "Keep asking" as the primary button.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { demoStartApproval, demoStartNoAskApproval } from '../fixtures';
import { StartApproval, type StartChoice } from './StartApproval';

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

const button = (text: string) => [...host.querySelectorAll('button')].find((b) => b.textContent === text) as HTMLButtonElement | undefined;
const click = (text: string) => { const b = button(text); if (!b) throw new Error(`no button ${text}`); act(() => { b.click(); }); };

describe('a request for a helper that acts without asking', () => {
  it('names the option and what it means, and offers no grant', () => {
    act(() => root.render(<StartApproval approval={demoStartNoAskApproval} start={demoStartNoAskApproval.start} onResolve={() => {}} />));
    const text = host.textContent ?? '';
    expect(text).toContain('How it acts');
    expect(text).toContain("Don't ask");
    expect(text).toContain('runs commands and changes files without asking you first');
    expect(text).toContain('your answer covers this start with exactly the launch options shown');
    expect(host.querySelector('[role=radiogroup]')).toBeNull();
    expect(host.querySelector('[data-grant-choice]')).toBeNull();
  });

  it('is allowed only after the confirmation, and "Keep asking" goes back to the card', () => {
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={demoStartNoAskApproval} start={demoStartNoAskApproval.start} onResolve={(ok, c) => answers.push([ok, c])} />));
    click('Allow');
    expect(answers).toEqual([]);
    const dialog = host.querySelector('[role=alertdialog]');
    expect(dialog?.textContent).toContain('Let the agent act without asking?');
    // The primary, focused button keeps asking: nothing is answered and the card is back.
    expect(document.activeElement?.textContent).toBe('Keep asking');
    click('Keep asking');
    expect(answers).toEqual([]);
    expect(host.querySelector('[role=alertdialog]')).toBeNull();
    click('Allow');
    click('Start without asking');
    expect(answers).toEqual([[true, { grant: 0, nested: false }]]);
  });

  it('says only what applies to an option that does not skip questions: no warning, no confirmation', () => {
    const harmless = { ...demoStartNoAskApproval.start, options: [{ id: 'fast', label: 'option-fast', skipsPermissions: false }] };
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={demoStartNoAskApproval} start={harmless} onResolve={(ok, c) => answers.push([ok, c])} />));
    const text = host.textContent ?? '';
    expect(text).toContain('your answer covers this start with exactly the launch options shown');
    expect(text).not.toContain('without asking');
    click('Allow');
    expect(answers).toEqual([[true, { grant: 0, nested: false }]]);
  });

  it('can be denied without the confirmation, and a request without options is allowed in one step as before', () => {
    const answers: [boolean, StartChoice][] = [];
    act(() => root.render(<StartApproval approval={demoStartNoAskApproval} start={demoStartNoAskApproval.start} onResolve={(ok, c) => answers.push([ok, c])} />));
    click('Deny');
    act(() => root.render(<StartApproval approval={demoStartApproval} start={demoStartApproval.start} onResolve={(ok, c) => answers.push([ok, c])} />));
    click('Allow');
    expect(answers).toEqual([[false, { grant: 0, nested: false }], [true, { grant: 0, nested: false }]]);
  });
});
