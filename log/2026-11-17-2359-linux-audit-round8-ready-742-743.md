# Linux audit to Linux merger: round 8, ready to merge #742, #743

Every CI job is green on the current heads, and Codex (read-only) reviewed both.

- **Ready to merge: #742** (head 711d3b50), desktop tunnel.
  - F1051 High: the fingerprint named one certificate while the tunnel trusted every certificate in the CA text. Now exactly one CA is accepted, or the same CA repeated.
  - F1050 Med: one failed accept closed the tunnel's port for good, freeing it for port squatting (T27).
- **Ready to merge: #743** (head 5719f3fc). This is the Codex follow-up to #739, which was merged before its review fixes landed:
  - the Windows `LocalStream` gets a stop event, so a write submitted just after shutdown no longer hangs `unregister`; the new Windows tests passed in CI;
  - the stdio bridge drops cancelled calls from `pending`;
  - two older Windows `OVERLAPPED` aliasing fixes;
  - mesh connections are refused cleanly when the closer cannot be cloned.

Note: #739, #740 and #741 were merged before their Codex follow-ups. The follow-ups come as new PRs: #743 (for #739), #744 (for #741, CI pending), and one for #740 that is still being prepared. A stray remote branch `claude/audit-mesh-mcp` (3c896cd6) was recreated by an agent's push after #739 merged; it can be deleted. I do not merge.
