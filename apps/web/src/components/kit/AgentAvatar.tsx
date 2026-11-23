import type { CSSProperties } from 'react';

// Who an agent is: its assigned colour (`accent`, a `var(--agent-*)` value from the model) as a soft square with a ring, and the first letter
// of its kind. It carries identity wherever the agent appears, instead of a coloured edge. Decorative beside the agent's name; give
// `label` (the agent's name) when the avatar stands alone. `needsYou` adds the amber dot.
export function AgentAvatar({ accent, kind, size = 'm', needsYou, label }: { accent: string; kind: string; size?: 's' | 'm' | 'l'; needsYou?: boolean; label?: string }) {
  const letter = Array.from(kind.trim())[0]?.toLocaleUpperCase() ?? '?';
  const a11y = label ? { role: 'img', 'aria-label': label } : { 'aria-hidden': true };
  return (
    <span className={`kit-avatar${size === 'm' ? '' : ` kit-avatar--${size}`}`} style={{ '--avatar': accent } as CSSProperties} {...a11y}>
      {letter}
      {needsYou ? <span className="kit-avatar__dot" /> : null}
    </span>
  );
}
