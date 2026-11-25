// What a person reads about a theme: names and problems, from locales/<lang>/web.ftl.
import { t } from '../i18n';
import { MAX_USER_THEMES } from './store';
import type { KitColour } from './tints';
import type { AnsiName, ChromeName, FloorItem, RegionName, RegionPart, SyntaxKey, TerminalKey } from './look';
import { MAX_NAME, type Theme, type ThemeIssue, type TokenName, isTokenName, kindOf, shownRatio } from './tokens';

export const themeName = (theme: Pick<Theme, 'name' | 'nameKey'>) => (theme.nameKey ? t(theme.nameKey) : theme.name);
export const tokenLabel = (name: TokenName) => t(`theme-token-${name}`);
// A colour the widget kit draws, as a person calls it: a token's own label, the red, a role's background or icon chip, an agent's badge.
export function kitColourLabel(c: KitColour): string {
  if (isTokenName(c)) return tokenLabel(c);
  if (c.startsWith('tint-') || c.startsWith('chip-')) return t(`theme-${c}`);
  if (c.startsWith('avatar-')) return t('theme-avatar', { colour: t(`theme-agent-colour-${c.slice('avatar-'.length)}`) });
  return t(`theme-colour-${c}`); // danger, on-danger, surface-raised
}

// Format 2's names, as a person reads them (locales/<lang>/web.ftl).
export const regionLabel = (r: RegionName) => t(`theme-region-${r}`);
export const regionPartLabel = (r: RegionName, p: RegionPart) => t(`theme-region-part-${p}`, { region: regionLabel(r) });
export const terminalLabel = (k: TerminalKey) => t(`theme-terminal-${k}`);
export const syntaxLabel = (k: SyntaxKey) => t(`theme-syntax-${k}`);
export const chromeLabel = (n: ChromeName) => t(`theme-chrome-${n}`);
export const chromeValueLabel = (n: ChromeName, v: string) => t(`theme-chrome-${n}-${v}`);
export const floorLabel = (f: FloorItem) => t(`theme-floor-row-${f}`);
export const floorReason = (f: FloorItem) => t(`theme-floor-${f}`);

// A problem as a sentence; one in a theme with both schemes says which.
export function issueText(i: ThemeIssue): string {
  const text = issueSentence(i);
  return i.scheme ? t('theme-issue-in-scheme', { scheme: t(`theme-${i.scheme}`), text }) : text;
}
function issueSentence(i: ThemeIssue): string {
  switch (i.code) {
    case 'name': return t('theme-error-name', { max: MAX_NAME });
    case 'unknown-field': return t('theme-error-unknown-field', { field: i.field });
    case 'unknown-token': return t('theme-error-unknown-token', { token: i.token });
    case 'derived-token': return t('theme-error-derived-token', { token: i.token, base: i.base });
    case 'bad-value': {
      const k = kindOf(i.token);
      const label = tokenLabel(i.token);
      return k.kind === 'px' ? t('theme-error-bad-size', { label, token: i.token, min: k.min, max: k.max })
        : t(k.kind === 'color' ? 'theme-error-bad-color' : 'theme-error-bad-font', { label, token: i.token });
    }
    case 'contrast': return t('theme-error-contrast', { fg: tokenLabel(i.fg), bg: tokenLabel(i.bg), ratio: shownRatio(i.ratio), min: i.min });
    // The same sentence as a surface pair; the background is a role's coloured background, the red is the fixed one.
    case 'tint-contrast': return t('theme-error-contrast', { fg: i.fg === 'danger' ? t('theme-colour-danger') : tokenLabel(i.fg), bg: t(`theme-tint-${i.tint}`), ratio: shownRatio(i.ratio), min: i.min });
    case 'kit-contrast': return t('theme-error-contrast', { fg: kitColourLabel(i.fg), bg: kitColourLabel(i.bg), ratio: shownRatio(i.ratio), min: i.min });
    // A control is seen, not read: its own sentence, naming the control.
    case 'control-contrast': return t('theme-error-control', { control: t(`theme-control-${i.control}`), fg: kitColourLabel(i.fg), bg: kitColourLabel(i.bg), ratio: shownRatio(i.ratio), min: i.min });
    case 'moved-token': return t('theme-error-moved-token', { field: i.field, token: i.token, to: i.to });
    case 'bad-field': return t(`theme-error-field-${i.expect}`, { field: i.field, min: i.min ?? 0, max: i.max ?? 0, values: i.values ?? '' });
    case 'unsafe': return t('theme-error-unsafe', { field: i.field });
    case 'floor': return t('theme-error-floor', { field: i.field, reason: floorReason(i.item) });
    case 'text-floor': return t('theme-error-text-floor', { field: i.field, min: i.min });
    case 'text-lifted': return t('theme-warning-text-lifted', { from: i.from, to: i.to });
    case 'inherits': return t('theme-error-inherits', { id: i.id });
    case 'region-contrast': return t(i.min < 4.5 ? 'theme-error-seen' : 'theme-error-contrast', {
      fg: i.fg === 'region-text' ? regionPartLabel(i.region, 'text') : tokenLabel(i.fg), bg: regionPartLabel(i.region, 'surface'), ratio: shownRatio(i.ratio), min: i.min,
    });
    case 'terminal-contrast': return t('theme-error-contrast', { fg: terminalLabel('foreground'), bg: terminalLabel('background'), ratio: shownRatio(i.ratio), min: i.min });
    case 'cursor-contrast': return t('theme-error-seen', { fg: terminalLabel('cursor'), bg: terminalLabel('background'), ratio: shownRatio(i.ratio), min: i.min });
    case 'ansi-contrast': return t('theme-error-seen', { fg: terminalLabel(i.colour as AnsiName), bg: terminalLabel('background'), ratio: shownRatio(i.ratio), min: i.min });
    case 'syntax-contrast': return t('theme-error-contrast', { fg: syntaxLabel(i.key), bg: regionPartLabel('code-blocks', 'surface'), ratio: shownRatio(i.ratio), min: i.min });
    default: return t(`theme-error-${i.code}`);
  }
}
export const fullText = () => t('theme-error-full', { max: MAX_USER_THEMES });
