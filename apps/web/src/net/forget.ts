// Why the hub did not forget a closed agent, in words the web's own texts have (the hub's error texts are not loaded here).
export function forgetReasonKey(code: string): string {
  switch (code) {
    case 'auth.forbidden': return 'closed-reason-forbidden';
    case 'agent.running': return 'closed-reason-running';
    case 'agent.not_found': return 'closed-reason-gone';
    default: return 'closed-reason-other';
  }
}

// Why the hub did not forget a machine (ADR-0073), in the web's own words.
export function forgetMachineReasonKey(code: string): string {
  switch (code) {
    case 'auth.forbidden': return 'forget-machine-reason-forbidden';
    case 'node.online': return 'forget-machine-reason-online';
    case 'node.unknown': return 'forget-machine-reason-gone';
    default: return 'forget-machine-reason-other';
  }
}
