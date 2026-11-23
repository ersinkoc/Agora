// A switch for a setting that is on or off. A native button with role="switch": Enter and Space flip it, and its state is aria-checked.
// Name it with `label` or with `labelledBy` (a SettingRow's label id). `risk`: turning it on lets an agent act without asking, so it is red
// when on (ADR-0102), and the screen that uses it keeps its own confirm step.
export function Toggle({ checked, onChange, label, labelledBy, describedBy, risk, disabled }: {
  checked: boolean; onChange: (next: boolean) => void; label?: string; labelledBy?: string; describedBy?: string; risk?: boolean; disabled?: boolean;
}) {
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} aria-labelledby={labelledBy} aria-describedby={describedBy}
      disabled={disabled} className={`kit-toggle${risk ? ' kit-toggle--risk' : ''}`} onClick={() => onChange(!checked)} />
  );
}
