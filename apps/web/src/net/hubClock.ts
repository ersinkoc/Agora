// The hub's clock as this window last heard it (`Welcome.server_time_ms`, ADR-0106), as an offset from this device's clock. Used where a
// time written by another device must not be believed past the hub's own now (themes/sync.ts). Until a hub says its time, this device's.
let offset = 0;
/** Notes the hub's time from its `Welcome` (0: an older hub that says none). */
export function noteHubTime(serverTimeMs: number) {
  if (Number.isFinite(serverTimeMs) && serverTimeMs > 0) offset = serverTimeMs - Date.now();
}
/** Now, by the hub's clock as far as it is known. */
export const hubNow = () => Date.now() + offset;
