import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from '../i18n';
import { LaunchSettings } from './LaunchSettings';
import { chooseSetting, chosenSettings, launchSettingsField, noSettings, plainValue, settingName, usableSettings, type LaunchSettingChoice } from './launchSettingsModel';
import { directoryMessage, launchMessageV4, signDirectory } from '../net/directory';
import { thisDevice } from '../net/approval';

const model: LaunchSettingChoice = { id: 'model', label: 'setting-model', values: [], defaultValue: '' };
const effort: LaunchSettingChoice = { id: 'effort', label: 'setting-effort', values: ['low', 'medium', 'high'], defaultValue: 'medium' };
const specs = [model, effort];

describe('start settings (ADR-0066)', () => {
  beforeEach(() => setLanguage('en'));

  it('sends nothing until a value is chosen: the agent keeps its own default', () => {
    expect(chosenSettings(noSettings, specs)).toEqual([]);
    expect(chosenSettings(chooseSetting(chooseSetting(noSettings, effort, 'high'), effort, ''), specs)).toEqual([]);
  });

  it('sends the chosen values in the order the kind declares them', () => {
    const s = chooseSetting(chooseSetting(noSettings, effort, 'high'), model, 'gpt-6.1-sol');
    expect(chosenSettings(s, specs)).toEqual([['model', 'gpt-6.1-sol'], ['effort', 'high']]);
  });

  it('never takes a value the machine did not list', () => {
    expect(chooseSetting(noSettings, effort, 'insane')).toEqual({});
    expect(chooseSetting({ effort: 'low' }, effort, 'insane')).toEqual({}); // the old choice does not stay as a stand-in
    // Even a forced state is filtered on the way out: a value outside the list, a setting the kind does not have.
    expect(chosenSettings({ effort: 'insane', mystery: 'x', model: '--flag' }, specs)).toEqual([]);
  });

  it('refuses what is not a plain token for a setting without a list', () => {
    for (const bad of ['-m', 'a b', 'a"b', 'a=b', '', 'x'.repeat(129), 'é']) expect(plainValue(bad)).toBe(false);
    for (const ok of ['gpt-6.1-sol', 'provider/model:1', 'opus', 'a@b+c']) expect(plainValue(ok)).toBe(true);
    expect(chooseSetting(noSettings, model, 'a b')).toEqual({});
  });

  it('cleans the list the hub relayed: bad ids and values are dropped, an emptied list is not "any value"', () => {
    const cleaned = usableSettings([
      { id: 'Model!', label: '', values: [], defaultValue: '' }, { id: 'effort', label: 'setting-effort', values: ['low', 'a b', '--x'], defaultValue: '' },
      { id: 'effort', label: '', values: ['high'], defaultValue: '' }, { id: 'tone', label: '', values: ['a b'], defaultValue: '' },
    ]);
    expect(cleaned).toEqual([{ id: 'effort', label: 'setting-effort', values: ['low'], defaultValue: '' }]);
  });

  it('names a setting in the window\'s words, or by its id', () => {
    expect(settingName(model, t)).toBe('Model');
    expect(settingName({ ...model, id: 'tone', label: 'no-such-message' }, t)).toBe('tone');
  });

  it('shows nothing for a kind without settings', () => {
    expect(renderToStaticMarkup(<LaunchSettings settings={[]} chosen={noSettings} onChange={() => {}} />)).toBe('');
  });

  it('offers the listed values, and the agent\'s own default first and selected', () => {
    const html = renderToStaticMarkup(<LaunchSettings settings={specs} chosen={noSettings} onChange={() => {}} />);
    expect(html).toContain('Model');
    expect(html).toContain('Effort');
    expect(html).toContain('The agent&#x27;s own default (medium)');
    for (const v of ['low', 'medium', 'high']) expect(html).toContain(`>${v}</button>`);
    expect(html).not.toContain('>max</button>');
    expect(html).toMatch(/aria-checked="true"[^>]*>The agent&#x27;s own default/);
    expect(html).toContain('<input');
  });

  it('marks the chosen value', () => {
    const html = renderToStaticMarkup(<LaunchSettings settings={[effort]} chosen={{ effort: 'high' }} onChange={() => {}} />);
    expect(html).toMatch(/aria-checked="true"[^>]*>high</);
  });
});

describe('what the device signs for a start with and without settings', () => {
  it('keeps the old request bytes when no setting is chosen, and signs v4 when one is', async () => {
    const device = await thisDevice({ load: async () => undefined, save: async () => {} });
    const none = await signDirectory(device, 'node', 'launch', '/w', 'codex', 'id:p', false, false, [], [], chosenSettings(noSettings, specs));
    const plain = directoryMessage('node', 'launch', '/w', 'codex', 'id:p', false, false, none.nonce, none.expiresAtMs);
    expect(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.publicKey, none.signature as BufferSource, plain as BufferSource)).toBe(true);
    const some = await signDirectory(device, 'node', 'launch', '/w', 'codex', 'id:p', false, false, [], [], chosenSettings({ effort: 'high' }, specs));
    const v1 = directoryMessage('node', 'launch', '/w', 'codex', 'id:p', false, false, some.nonce, some.expiresAtMs);
    expect(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.publicKey, some.signature as BufferSource, v1 as BufferSource)).toBe(false);
    const v4 = launchMessageV4('node', '/w', 'codex', 'id:p', false, false, [], [], [['effort', 'high']], some.nonce, some.expiresAtMs);
    expect(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.publicKey, some.signature as BufferSource, v4 as BufferSource)).toBe(true);
  });

  it('puts no `settings` field into the request when none is chosen', () => {
    expect(launchSettingsField([])).toEqual({});
    expect(launchSettingsField([['model', 'x'], ['effort', 'high']])).toEqual({ settings: [{ id: 'model', value: 'x' }, { id: 'effort', value: 'high' }] });
  });
});
