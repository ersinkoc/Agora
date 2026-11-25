// @vitest-environment jsdom
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../../i18n';
import { AgentAvatar, Button, Callout, Card, EmptyState, KeyValue, ListRow, Pill, SegmentedControl, SettingRow, StatTile, StatusMark, StatusPill, Tag, Toggle, Tooltip } from '.';
import { KitGallery } from './KitGallery';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
const render = (el: React.ReactNode) => act(() => root.render(el));
const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T & HTMLElement>(sel) as T & HTMLElement;
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })); });

describe('widget kit', () => {
  it('buttons are real buttons; danger is a button that says it is destructive by its variant, not by a role of its own', () => {
    render(<><Button variant="danger">Forget</Button><Button variant="primary" size="sm">Start</Button><Button disabled>Open</Button></>);
    const [danger, primary, off] = Array.from(host.querySelectorAll('button'));
    expect(danger?.tagName).toBe('BUTTON');
    expect(danger?.getAttribute('type')).toBe('button');
    expect(danger?.getAttribute('role')).toBeNull(); // its role is button: Enter and Space press it, a screen reader says "button"
    expect(danger?.className).toContain('kit-btn--danger');
    expect(danger?.textContent).toBe('Forget');
    expect(primary?.className).toContain('kit-btn--sm');
    expect(off?.disabled).toBe(true);
  });

  it('a status pill says its state in words with a shape; failed is red with a cross and the word Failed (ADR-0102)', () => {
    render(<><StatusPill state="working" /><StatusPill state="finished" failed /><StatusPill state="needs-you" /></>);
    const pills = Array.from(host.querySelectorAll('.kit-pill'));
    expect(pills.map((p) => p.textContent)).toEqual(['Working', 'Failed', 'Needs you']);
    expect(pills[0]?.className).toContain('kit-tone-working');
    expect(pills[1]?.className).toContain('kit-tone-danger');
    expect(pills[1]?.querySelectorAll('circle, path').length).toBe(2); // the cross in a ring
    for (const p of pills) expect(p.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    setLanguage('tr');
    render(<StatusPill state="finished" failed />);
    expect(q('.kit-pill').textContent).not.toMatch(/fleet-failed|Failed/);
  });

  it('a status mark is the shape alone in its kit colour, failed in red, with the word for screen readers', () => {
    render(<><StatusMark state="finished" failed /><StatusMark state="working" /></>);
    const [failed, working] = Array.from(host.querySelectorAll('.kit-mark'));
    expect(failed?.className).toContain('kit-tone-danger');
    expect(failed?.querySelector('.sr-only')?.textContent).toBe('Failed');
    expect(working?.className).toContain('kit-tone-working');
  });

  it('pills and tags are plain text; a tag is never a state colour', () => {
    render(<><Pill tone="accent">Shared</Pill><Tag mono>0.3.34</Tag></>);
    expect(q('.kit-pill').className).toContain('kit-tone-accent');
    expect(q('.kit-tag').className).toBe('kit-tag kit-tag--mono');
  });

  it('an avatar shows the kind letter in the agent colour; decorative beside a name, named when it stands alone', () => {
    render(<><AgentAvatar accent="var(--agent-teal)" kind="codex" /><AgentAvatar accent="var(--agent-amber)" kind="claude" label="claude · amber" needsYou size="l" /></>);
    const [a, b] = Array.from(host.querySelectorAll<HTMLElement>('.kit-avatar'));
    expect(a?.textContent).toBe('C');
    expect(a?.getAttribute('aria-hidden')).toBe('true');
    expect(a?.style.getPropertyValue('--avatar')).toBe('var(--agent-teal)');
    expect(b?.getAttribute('role')).toBe('img');
    expect(b?.getAttribute('aria-label')).toBe('claude · amber');
    expect(b?.querySelector('.kit-avatar__dot')).not.toBeNull();
    expect(b?.className).toContain('kit-avatar--l');
  });
  it('makes an avatar letter visible when hidden text leads the kind', () => {
    render(<AgentAvatar accent="var(--agent-teal)" kind={'\u3164\u2800codex'} label="agent" />);
    const avatar = document.querySelector('.kit-avatar') as HTMLElement;
    expect([...avatar.textContent ?? ''].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(avatar.textContent).toBe('C');
  });
  it('verifies avatar letters after bidi controls and for an all-hidden kind', () => {
    render(<><AgentAvatar accent="var(--agent-teal)" kind={'\u202eGrüße'} /><AgentAvatar accent="var(--agent-teal)" kind={'\u2066\u3164'} /></>);
    expect([...document.querySelectorAll('.kit-avatar')].map((el) => el.textContent)).toEqual(['G', '?']);
  });

  it('a card is a group, or one pressable button that says when it is selected', () => {
    render(<><Card label="laptop">x</Card><Card selected onClick={() => {}}>y</Card></>);
    expect(q('div.kit-card').getAttribute('role')).toBe('group');
    const b = q('button.kit-card');
    expect(b.getAttribute('aria-pressed')).toBe('true');
    expect(b.hasAttribute('data-selected')).toBe(true);
  });

  it('a list row marks the current one with aria-current, as a link or a button', () => {
    render(<><ListRow label="one" href="#/a" current /><ListRow label="two" onClick={() => {}} current /><ListRow label="three" onClick={() => {}} /></>);
    expect(q('a.kit-row').getAttribute('aria-current')).toBe('page');
    const buttons = host.querySelectorAll('button.kit-row');
    expect(buttons[0]?.getAttribute('aria-current')).toBe('true');
    expect(buttons[1]?.hasAttribute('aria-current')).toBe(false);
  });

  it('a setting row names its control once, with its hint as the description', () => {
    render(<SettingRow label="Notifications" hint="Names only">{(l, h) => <Toggle checked={false} onChange={() => {}} labelledBy={l} describedBy={h} />}</SettingRow>);
    const sw = q('[role=switch]');
    expect(document.getElementById(sw.getAttribute('aria-labelledby') ?? '')?.textContent).toBe('Notifications');
    expect(document.getElementById(sw.getAttribute('aria-describedby') ?? '')?.textContent).toBe('Names only');
  });

  it('a toggle is a native switch button: a press flips aria-checked; risk only changes its colour', () => {
    function Host() { const [on, set] = useState(false); return <Toggle checked={on} onChange={set} label="Notifications" risk />; }
    render(<Host />);
    const sw = q<HTMLButtonElement>('button[role=switch]');
    expect(sw.getAttribute('type')).toBe('button'); // a <button>: the browser presses it with Enter and Space
    expect(sw.tabIndex).toBe(0);
    expect(sw.getAttribute('aria-label')).toBe('Notifications');
    expect(sw.getAttribute('aria-checked')).toBe('false');
    act(() => sw.click());
    expect(sw.getAttribute('aria-checked')).toBe('true');
    expect(sw.className).toContain('kit-toggle--risk');
  });

  it('a segmented control is a radio group with one tab stop; arrows, Home and End move and choose, mirrored right to left', () => {
    const seen: string[] = [];
    function Host() { const [v, set] = useState('b'); return <SegmentedControl label="Group by" value={v} onChange={(x) => { seen.push(x); set(x); }} items={[{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }, { value: 'c', label: 'C' }]} />; }
    render(<Host />);
    const group = q('[role=radiogroup]');
    expect(group.getAttribute('aria-label')).toBe('Group by');
    const radios = () => Array.from(host.querySelectorAll<HTMLButtonElement>('[role=radio]'));
    const checked = () => radios().findIndex((r) => r.getAttribute('aria-checked') === 'true');
    expect(radios().map((r) => r.tabIndex)).toEqual([-1, 0, -1]);
    key(radios()[1]!, 'ArrowRight');
    expect(checked()).toBe(2);
    expect(document.activeElement).toBe(radios()[2]);
    key(radios()[2]!, 'ArrowRight'); // wraps
    expect(checked()).toBe(0);
    key(radios()[0]!, 'End');
    expect(checked()).toBe(2);
    key(radios()[2]!, 'Home');
    expect(checked()).toBe(0);
    key(radios()[0]!, 'ArrowUp');
    expect(checked()).toBe(2);
    expect(radios().map((r) => r.tabIndex)).toEqual([-1, -1, 0]);
    group.setAttribute('dir', 'rtl');
    group.style.direction = 'rtl';
    key(radios()[2]!, 'ArrowLeft'); // in a right-to-left page, left is forward
    expect(checked()).toBe(0);
    act(() => radios()[1]!.click());
    expect(seen.at(-1)).toBe('b');
  });

  it('a segmented control can be off as a whole, and is red when its chosen item lets an agent act without asking', () => {
    const items = [{ value: 'ask', label: 'Ask' }, { value: 'skip', label: "Don't ask" }];
    render(<SegmentedControl label="How it acts" value="skip" risk disabled onChange={() => {}} items={items} />);
    const group = q('[role=radiogroup]');
    expect(group.className).toBe('kit-seg kit-seg--risk');
    expect(group.getAttribute('aria-disabled')).toBe('true');
    for (const r of Array.from(host.querySelectorAll<HTMLButtonElement>('[role=radio]'))) expect(r.disabled).toBe(true);
    render(<SegmentedControl label="How it acts" value="ask" onChange={() => {}} items={items} />);
    expect(q('[role=radiogroup]').className).toBe('kit-seg');
    expect(q('[role=radiogroup]').hasAttribute('aria-disabled')).toBe(false);
  });

  it('a stat tile is a pressable filter only when it filters; a zero is not tinted', () => {
    render(<><StatTile value={2} tone="needs" label="Needs you" /><StatTile value={0} tone="needs" label="Needs you" /><StatTile value={3} label="Working" pressed onPress={() => {}} /></>);
    const [two, zero] = Array.from(host.querySelectorAll('div.kit-stat'));
    expect(two?.className).toContain('kit-stat--tinted kit-tone-needs');
    expect(zero?.className).toContain('kit-stat--zero');
    expect(zero?.className).not.toContain('tinted');
    expect(q('button.kit-stat').getAttribute('aria-pressed')).toBe('true');
  });

  it('a callout is a section named by its title, an alert only when asked; an empty state is a status with one action', () => {
    render(<><Callout tone="danger" icon="warn" title="Could not send" alert actions={<Button>Retry</Button>}>body</Callout><EmptyState icon="chat" title="No messages yet" line="Later" action={<Button>Start</Button>} /></>);
    const c = q('section.kit-callout');
    expect(c.getAttribute('role')).toBe('alert');
    expect(document.getElementById(c.getAttribute('aria-labelledby') ?? '')?.textContent).toBe('Could not send');
    expect(c.className).toContain('kit-tone-danger');
    expect(c.querySelector('.kit-chip')?.getAttribute('aria-hidden')).toBe('true');
    const e = q('.kit-empty');
    expect(e.getAttribute('role')).toBe('status');
    expect(e.querySelectorAll('button').length).toBe(1);
  });

  it('key-value facts are a description list', () => {
    render(<KeyValue items={[{ label: 'Device name', value: 'web-1', mono: true }, { label: 'Machine', value: 'laptop' }]} />);
    expect(Array.from(host.querySelectorAll('dl dt')).map((d) => d.textContent)).toEqual(['Device name', 'Machine']);
    expect(q('dl dd').className).toBe('kit-kv__mono');
  });

  describe('tooltip', () => {
    const over = (el: Element) => act(() => { el.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body })); });
    const out = (el: Element) => act(() => { el.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body })); });
    const esc = () => act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })); });
    const later = () => act(() => { vi.advanceTimersByTime(500); });
    beforeEach(() => { vi.useFakeTimers(); });
    afterEach(() => { vi.useRealTimers(); });
    const setup = () => {
      render(<><span id="own">Own hint</span><Tooltip text="Copy agent address"><button type="button" aria-label="Copy" aria-describedby="own">c</button></Tooltip><button type="button">elsewhere</button></>);
      return { b: q('button[aria-label=Copy]'), tip: q('[role=tooltip]') };
    };
    it('adds itself to the control\'s description, keeping the one it had and the control\'s own name', () => {
      const { b, tip } = setup();
      expect(b.getAttribute('aria-describedby')).toBe(`own ${tip.id}`);
      expect(b.getAttribute('aria-label')).toBe('Copy');
      expect(tip.hidden).toBe(true);
    });
    it('stays open while the control has focus, though the pointer leaves', () => {
      const { b, tip } = setup();
      act(() => b.focus());
      over(b); out(b); later();
      expect(tip.hidden).toBe(false);
      act(() => b.blur());
      expect(tip.hidden).toBe(true);
    });
    it('stays open while hovered, though focus leaves; closes a moment after the pointer leaves', () => {
      const { b, tip } = setup();
      over(b);
      act(() => b.focus());
      act(() => b.blur());
      expect(tip.hidden).toBe(false);
      out(b);
      expect(tip.hidden).toBe(false); // the pointer may be on its way onto the tooltip
      later();
      expect(tip.hidden).toBe(true);
    });
    it('Esc closes it while hovered and focused, until the next hover or focus', () => {
      const { b, tip } = setup();
      over(b);
      act(() => b.focus());
      esc();
      expect(tip.hidden).toBe(true);
      act(() => b.blur());
      act(() => b.focus());
      expect(tip.hidden).toBe(false);
    });
  });

  for (const lang of ['en', 'tr']) {
    it(`the gallery shows the kit with every text from the ${lang} locale`, () => {
      setLanguage(lang);
      render(<KitGallery />);
      expect(host.textContent).not.toMatch(/\bkit-[a-z-]+|fleet-group-|state-[a-z]|Unknown variable/);
      expect(host.querySelectorAll('[role=radiogroup]').length).toBeGreaterThanOrEqual(3);
      for (const r of Array.from(host.querySelectorAll('[role=radiogroup], [role=switch]'))) {
        const named = r.getAttribute('aria-label') ?? document.getElementById(r.getAttribute('aria-labelledby') ?? '')?.textContent;
        expect(named, r.outerHTML.slice(0, 80)).toBeTruthy();
      }
    });
  }
});
