// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { paletteExtras, type PaletteExtras } from '../net/paletteExtras';
import type { MessageModel } from '../net/view';
import { themeName } from '../themes/messages';
import { CommandPalette } from './CommandPalette';
import { TeamChat } from './TeamChat';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const noop = () => undefined;
const extras = (machine: string, project: string): PaletteExtras => ({
  machines: [{ id: 'n1', name: machine }], projects: [{ id: 'p1', name: project }], hub: true, phone: false,
  passwordSet: false, canPair: false, go: noop, showProject: noop, showPairing: noop,
});
const message = (toMachine: string): MessageModel => ({ from: 'codex', to: 'claude', body: 'done', atMs: 1, crossMachine: true, toMachine });

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); document.body.replaceChildren(); });

describe('palette and theme labels make hidden characters visible', () => {
  it('writes hidden characters in generic command-palette labels as escapes', () => {
    act(() => root.render(<CommandPalette items={[{ id: 'x', label: 'op\u202een', run: noop }]} onClose={noop} />));
    expect(document.querySelector('.palette__label')?.textContent).toBe('op\\u{202e}en');
  });

  it('writes hidden characters in machine jump labels as escapes', () => {
    expect(paletteExtras(extras('lap\u3164top', 'shop'))[0]?.label).toBe(t('palette-open-machine', { name: 'lap\\u{3164}top' }));
  });

  it('writes hidden characters in project jump labels as escapes', () => {
    expect(paletteExtras(extras('laptop', 'sh\u2066op'))[1]?.label).toBe(t('palette-open-project', { name: 'sh\\u{2066}op' }));
  });

  it('writes hidden characters in user theme names as escapes', () => {
    expect(themeName({ name: 'ni\u034fght' })).toBe('ni\\u{34f}ght');
  });

  it('writes hidden characters in team-message destination machines as escapes', () => {
    expect(renderToStaticMarkup(<TeamChat messages={[message('lap\u200btop')]} now={1} />)).toContain('lap\\u{200b}top');
  });
});

describe('palette and theme label controls', () => {
  it('keeps an ordinary command-palette label and action id', () => {
    act(() => root.render(<CommandPalette items={[{ id: 'open', label: 'Open', run: noop }]} onClose={noop} />));
    expect(document.querySelector('.palette__label')?.textContent).toBe('Open');
    expect(document.querySelector('[role=option]')?.id).toBe('palette-open');
  });

  it('keeps an ordinary machine jump label and raw route id', () => {
    const item = paletteExtras(extras('laptop', 'shop'))[0];
    expect(item?.label).toBe(t('palette-open-machine', { name: 'laptop' }));
    expect(item?.id).toBe('machine-n1');
  });

  it('keeps an ordinary project jump label and raw project id', () => {
    const item = paletteExtras(extras('laptop', 'shop'))[1];
    expect(item?.label).toBe(t('palette-open-project', { name: 'shop' }));
    expect(item?.id).toBe('project-p1');
  });

  it('keeps an ordinary user theme name', () => {
    expect(themeName({ name: 'Night' })).toBe('Night');
  });

  it('keeps an ordinary destination machine and message body', () => {
    const html = renderToStaticMarkup(<TeamChat messages={[message('server')]} now={1} />);
    expect(html).toContain('server');
    expect(html).toContain('done');
  });
});

const verifierEdges = [
  ['\u2069', '\\u{2069}'],
  ['\u0085', '\\u{85}'],
  ['\u{f0000}', '\\u{f0000}'],
] as const;

describe('palette and theme label verifier edges', () => {
  it('escapes other hidden characters in generic palette labels without changing action ids', () => {
    for (const [hidden, escaped] of verifierEdges) {
      act(() => root.render(<CommandPalette items={[{ id: 'open', label: `op${hidden}en`, run: noop }]} onClose={noop} />));
      expect(document.querySelector('.palette__label')?.textContent).toBe(`op${escaped}en`);
      expect(document.querySelector('[role=option]')?.id).toBe('palette-open');
    }
  });

  it('escapes other hidden characters in machine jumps without changing route ids', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const item = paletteExtras(extras(`ma${hidden}chine`, 'shop'))[0];
      expect(item?.label).toBe(t('palette-open-machine', { name: `ma${escaped}chine` }));
      expect(item?.id).toBe('machine-n1');
    }
  });

  it('escapes other hidden characters in project jumps without changing project ids', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const item = paletteExtras(extras('laptop', `pro${hidden}ject`))[1];
      expect(item?.label).toBe(t('palette-open-project', { name: `pro${escaped}ject` }));
      expect(item?.id).toBe('project-p1');
    }
  });

  it('escapes other hidden characters in user theme names', () => {
    for (const [hidden, escaped] of verifierEdges) expect(themeName({ name: `ni${hidden}ght` })).toBe(`ni${escaped}ght`);
  });

  it('escapes other hidden characters in destination machines without changing message bodies', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<TeamChat messages={[message(`ma${hidden}chine`)]} now={1} />);
      expect(html).toContain(`ma${escaped}chine`);
      expect(html).toContain('done');
    }
  });
});
