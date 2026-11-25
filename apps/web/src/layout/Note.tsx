import type { ReactNode } from 'react';

// A pane's short message in its middle (nothing waiting, an unknown panel, an agent that is gone).
export function Note({ children }: { children: ReactNode }) {
  return <div className="grid min-h-0 flex-1 place-items-center p-4 text-center"><p className="muted m-0 max-w-xs">{children}</p></div>;
}
