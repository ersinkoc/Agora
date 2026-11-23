import { useId, type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';
import type { Tone } from './kit';

// One Settings section as a card (redesign PR 8): an icon chip, a title, one line on what the section is for and, at the end, its state
// as a pill or its one action; then its setting rows (kit SettingRow, a form, or a list), each under a hairline. `id` is the section's
// address (#/device/<id>), so a deep link and the side index land on it.
export function SettingsCard({ id, icon, tone = 'accent', title, description, end, children }: {
  id?: string; icon: IconName; tone?: Tone; title: ReactNode; description?: ReactNode; end?: ReactNode; children?: ReactNode;
}) {
  const titleId = useId();
  return (
    <section id={id ? `settings-${id}` : undefined} aria-labelledby={titleId} className="set-card">
      <div className="set-card__head">
        <span aria-hidden className={`kit-chip set-card__icon kit-tone-${tone}`}><Icon name={icon} /></span>
        <div className="set-card__title">
          <h3 id={titleId} className="set-card__h">{title}</h3>
          {description ? <p className="set-card__desc">{description}</p> : null}
        </div>
        {end ? <div className="set-card__end">{end}</div> : null}
      </div>
      {children}
    </section>
  );
}

// A part of a card that is not a setting row: a longer explanation with its action, a command to copy, a notice.
export function SettingsBlock({ children }: { children: ReactNode }) {
  return <div className="set-card__block">{children}</div>;
}

// The system a machine reports (`linux`, `macos`, `windows`), or the one a browser's name says ("Chrome · macOS"), as an outline icon.
export function systemIcon(system: string): IconName {
  const s = system.toLowerCase();
  if (/windows/.test(s)) return 'windows';
  if (/mac|darwin|ios|iphone|ipad/.test(s)) return 'apple';
  if (/android/.test(s)) return 'phone';
  if (/linux|bsd/.test(s)) return 'linux';
  return 'globe';
}

// The square tile that holds a system icon at the start of a list row.
export function SystemTile({ system }: { system: string }) {
  return <span aria-hidden className="set-os"><Icon name={systemIcon(system)} /></span>;
}
