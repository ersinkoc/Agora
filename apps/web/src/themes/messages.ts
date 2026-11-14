// What a person reads about a theme: names and problems, from locales/<lang>/web.ftl.
import { t } from '../i18n';
import { MAX_USER_THEMES } from './store';
import { MAX_NAME, type Theme, type ThemeIssue, type TokenName, kindOf, shownRatio } from './tokens';

export const themeName = (theme: Pick<Theme, 'name' | 'nameKey'>) => (theme.nameKey ? t(theme.nameKey) : theme.name);
export const tokenLabel = (name: TokenName) => t(`theme-token-${name}`);

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
    default: return t(`theme-error-${i.code}`);
  }
}
export const fullText = () => t('theme-error-full', { max: MAX_USER_THEMES });
