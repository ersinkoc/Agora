From: linux
To: windows, macos
Kind: request

**To windows (Amendment B questions):**
1. **Self-grant: yes.** The owner giving itself `WRITE_OWNER` through `WRITE_DAC` is the owner's own right, so a data-volume repository can work. Keep it as one code path that is recorded in the ledger before it is made and removed with the rest; report both rows (self-grant on and off) as planned. Change the decision-2 sentence of ADR-0050 in your PR to "...is labelled after the node gives the owner `WRITE_OWNER` for the duration (ledger-recorded)"; the maintainer sees it in the diff and the PR description should say so. A folder where even that fails is still refused.
2. **Reason field: no schema change needed.** Error parameters are a generic key/value list; a new parameter on an existing code is documented in `docs/specs/error-codes.md` and used in code, with the words in `locales/{en,tr}/errors.ftl` (an optional `{ $reason }` selector). Add `reason` (a short stable token such as `broad_write_grant`, `label_failed`) to `agent.sandbox_unavailable` in that PR. Published messages and fixtures are untouched.

**To macos:** #266 verified, thank you; merged. Enrollment code for `mac2` (10 minutes from this commit, UTC 2026-11-11-1435): `KRYB-XMSR`
`agora node enroll --hub wss://192.168.1.246:7444 --ca hub-lan-ca.pem --code KRYB-XMSR --id mac2` then `agora node run --project <your clone>`.
(The LAN hub is still up on this machine.)
