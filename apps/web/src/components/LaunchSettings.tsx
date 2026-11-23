import { t } from '../i18n';
import { allowed, chooseSetting, settingName, type LaunchSettingChoice, type SettingsState } from './launchSettingsModel';

// The kit's segmented look (visual-language.md §3), wrapping when a list is long (an effort with six values on a phone); every value
// stays its own tab stop, as before.
const item = 'kit-seg__item';

// ADR-0066: the model and effort a kind takes. A setting with a list is a choice among exactly those values; one without (a model name the
// tool checks itself) is a text box. Both start on "the agent's own default", which sends nothing. Render only: the choice is the parent's state.
export function LaunchSettings({ settings, chosen, onChange, disabled }: {
  settings: readonly LaunchSettingChoice[];
  chosen: SettingsState;
  onChange: (next: SettingsState) => void;
  disabled?: boolean;
}) {
  if (settings.length === 0) return null;
  return <div className="grid gap-3">
    {settings.map((spec) => {
      const name = settingName(spec, t);
      const value = chosen[spec.id] ?? '';
      const own = spec.defaultValue ? t('setting-default-is', { value: spec.defaultValue }) : t('setting-default');
      return <div key={spec.id} className="grid gap-1.5" data-setting={spec.id}>
        <h2 className="m-0 text-strong font-semibold normal-case tracking-normal text-ink">{name}</h2>
        {spec.values.length > 0
          ? <div className="kit-seg flex-wrap justify-self-start" role="radiogroup" aria-label={name}>
            <button type="button" role="radio" aria-checked={value === ''} disabled={disabled} className={item} onClick={() => onChange(chooseSetting(chosen, spec, ''))}>{own}</button>
            {spec.values.map((v) => <button key={v} type="button" role="radio" aria-checked={value === v} disabled={disabled} className={item} onClick={() => onChange(chooseSetting(chosen, spec, v))}>{v}</button>)}
          </div>
          : <input value={value} disabled={disabled} maxLength={128} spellCheck={false} autoCapitalize="off" autoComplete="off" className="min-w-0 font-mono text-body" aria-label={name} placeholder={own}
            onChange={(e) => { const v = e.target.value; if (v === '' || allowed(spec, v)) onChange(chooseSetting(chosen, spec, v)); }} />}
      </div>;
    })}
  </div>;
}
