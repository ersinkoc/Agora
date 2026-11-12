// Why the hub did not forget a closed agent, in words the web's own texts have (the hub's error texts are not loaded here).
export function forgetReasonKey(code: string): string {
  switch (code) {
    case 'auth.forbidden': return 'closed-reason-forbidden';
    case 'agent.running': return 'closed-reason-running';
    case 'agent.not_found': return 'closed-reason-gone';
    default: return 'closed-reason-other';
  }
}
