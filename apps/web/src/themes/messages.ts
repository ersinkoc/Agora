// What a person reads about a theme: names and problems, from locales/<lang>/web.ftl.
import { t } from '../i18n';
import { MAX_USER_THEMES } from './store';
import type { KitColour } from './tints';
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

export function issueText(i: ThemeIssue): string {
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
    default: return t(`theme-error-${i.code}`);
  }
}
export const fullText = () => t('theme-error-full', { max: MAX_USER_THEMES });
