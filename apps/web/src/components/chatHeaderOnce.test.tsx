// @vitest-environment jsdom
import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { Focus } from '../screens';

// Redesign PR 12: a live chat agent's state and tokens are said once, in its session bar; the full-size view's header leaves them out.
// A terminal agent has no session bar, and a closed chat agent's bar has no state (its "failed" lives in the header), so both keep theirs.
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n/a', name: 'claude · amber', accent: '#fff', machine: 'm', workspace: '/w', state: 'idle', preview: '', tokens: 215, ...over });
const view = (a: AgentTileModel) => renderToStaticMarkup(<Focus agent={a} onBack={() => {}} />);
const header = (html: string) => html.slice(0, html.indexOf('</header>'));

beforeEach(() => setLanguage('en'));

describe('a chat agent in the full-size view', () => {
  it('shows its state once (the session pill) and its tokens once', () => {
    const html = view(agent({ terminal: false, session: { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 0, contextWindow: 0 } }));
    expect(html.match(/215 tokens used/g)).toHaveLength(1);
    expect(html.match(/data-state-pill=/g)).toHaveLength(1);
    expect(header(html)).not.toContain('class="state state--');
    expect(header(html)).not.toContain('tokens used');
  });
  it('keeps the header state of a closed chat agent, whose session bar shows none', () => {
    const html = view(agent({ terminal: false, closed: true }));
    expect(header(html)).toContain('class="state state--idle"');
    expect(html).not.toContain('data-state-pill=');
    expect(html.match(/215 tokens used/g)).toHaveLength(1);
  });
  it('leaves a terminal agent as it was: state and tokens in the header', () => {
    const html = view(agent({ state: 'working' }));
    expect(header(html)).toContain('class="state state--working"');
    expect(header(html)).toContain('215 tokens used');
    expect(html.match(/215 tokens used/g)).toHaveLength(1);
  });

  it('makes full-size agent and location labels visible', () => {
    const html = header(view(agent({ name: 'clau\u3164de\u202e', machine: 'lap\u034ftop', project: 'lo\u2066gin', branch: 'fi\ue000x' })));
    expect(html).toContain('clau\\u{3164}de\\u{202e}');
    expect(html).toContain('lap\\u{34f}top');
    expect(html).toContain('lo\\u{2066}gin');
    expect(html).toContain('fi\\u{e000}x');
  });

  it('verifies full-size labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u200d', '\\u{200d}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const html = header(view(agent({ name: `co${char}dex`, machine: `ma${char}chine`, project: `pro${char}ject`, branch: `br${char}anch` })));
      for (const [start, end] of [['co', 'dex'], ['ma', 'chine'], ['pro', 'ject'], ['br', 'anch']] as const) expect(html).toContain(`${start}${escaped}${end}`);
    }
  });
});
