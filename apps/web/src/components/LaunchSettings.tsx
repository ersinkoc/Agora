import { t } from '../i18n';
import { allowed, chooseSetting, settingName, type LaunchSettingChoice, type SettingsState } from './launchSettingsModel';

const chip = 'inline-flex items-center gap-2 rounded-xs border px-2.5 py-1.5 text-[0.8125rem] transition-colors duration-150 disabled:opacity-50';
const chipOn = 'border-accent bg-surface-2 text-ink ring-1 ring-accent';
const chipOff = 'border-line bg-surface-0 text-muted hover:bg-surface-2 hover:text-ink';

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
        <h2 className="m-0">{name}</h2>
        {spec.values.length > 0
          ? <div className="flex flex-wrap gap-1.5" role="radiogroup" aria-label={name}>
            <button type="button" role="radio" aria-checked={value === ''} disabled={disabled} className={`${chip} ${value === '' ? chipOn : chipOff}`} onClick={() => onChange(chooseSetting(chosen, spec, ''))}>{own}</button>
            {spec.values.map((v) => <button key={v} type="button" role="radio" aria-checked={value === v} disabled={disabled} className={`${chip} ${value === v ? chipOn : chipOff}`} onClick={() => onChange(chooseSetting(chosen, spec, v))}>{v}</button>)}
          </div>
          : <input value={value} disabled={disabled} maxLength={128} spellCheck={false} autoCapitalize="off" autoComplete="off" className="min-w-0 font-mono text-[0.8125rem]" aria-label={name} placeholder={own}
            onChange={(e) => { const v = e.target.value; if (v === '' || allowed(spec, v)) onChange(chooseSetting(chosen, spec, v)); }} />}
      </div>;
    })}
  </div>;
}
