// ADR-0066: what the start window offers from a kind's launch settings (model, effort), and the pairs a start sends and signs.
// The machine says what each kind takes; this window offers nothing else. "The agent's own default" is always the choice that is
// preselected, and it sends nothing.
export interface LaunchSettingChoice {
  id: string;
  /** A Fluent id; one this window does not have shows `id`. */
  label: string;
  /** The values the kind takes, in order. Empty: any plain value (a model name the tool itself checks). */
  values: string[];
  /** What the tool uses when nothing is chosen, for display only. */
  defaultValue: string;
}

/** The chosen value per setting id; a setting with no entry is left to the agent's own default. */
export type SettingsState = Readonly<Record<string, string>>;
export const noSettings: SettingsState = {};

/** A value the machine accepts as a plain token (the same rule as the node's `valid_setting_value`; the node stays the judge). */
export const plainValue = (v: string) => v.length > 0 && new TextEncoder().encode(v).length <= 128 && /^[A-Za-z0-9][A-Za-z0-9._:/@+-]*$/.test(v);
const usableId = (id: string) => /^[a-z0-9-]{1,32}$/.test(id);

/** The settings a kind lists, cleaned: an id this window may sign, listed once, only values that are plain tokens. A list whose values were
 * all dropped is dropped too (it must not turn into "any value"). */
export function usableSettings(specs: readonly LaunchSettingChoice[]): LaunchSettingChoice[] {
  const seen = new Set<string>();
  const out: LaunchSettingChoice[] = [];
  for (const s of specs) {
    if (!usableId(s.id) || seen.has(s.id)) continue;
    const values = s.values.filter(plainValue);
    if (s.values.length > 0 && values.length === 0) continue;
    seen.add(s.id);
    out.push({ ...s, values });
  }
  return out;
}

/** Whether `value` may be sent for `spec`: a listed value, or for a setting with no list a plain token. */
export const allowed = (spec: LaunchSettingChoice, value: string) => (spec.values.length > 0 ? spec.values.includes(value) : plainValue(value));

/** Choosing a value; the empty value (or the agent's default) removes the choice. A value the machine did not list is not taken. */
export function chooseSetting(s: SettingsState, spec: LaunchSettingChoice, value: string): SettingsState {
  const { [spec.id]: _gone, ...rest } = s;
  return value !== '' && allowed(spec, value) ? { ...rest, [spec.id]: value } : rest;
}

/** The pairs a start sends and signs: in the order the kind declares them, only settings it lists, only values it allows. Nothing chosen: `[]`
 * (the start is then byte for byte the one without settings). */
export function chosenSettings(s: SettingsState, specs: readonly LaunchSettingChoice[]): [string, string][] {
  return usableSettings(specs).flatMap((spec) => {
    const v = s[spec.id];
    return v !== undefined && allowed(spec, v) ? [[spec.id, v] as [string, string]] : [];
  });
}

/** A setting's name in the window's own words; a label this window does not have shows the id. */
export function settingName(spec: LaunchSettingChoice, translate: (id: string) => string): string {
  const said = spec.label ? translate(spec.label) : '';
  return said && said !== spec.label ? said : spec.id;
}

/** The `settings` part of an `AgentLaunch`: absent when nothing is chosen, so that request is byte for byte the one an older machine takes. */
export const launchSettingsField = (pairs: readonly (readonly [string, string])[]) => (pairs.length > 0 ? { settings: pairs.map(([id, value]) => ({ id, value })) } : {});
