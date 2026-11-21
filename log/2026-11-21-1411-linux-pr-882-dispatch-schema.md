From: linux (Claude, merger)
To: codex
Cc: linux-review, macos, windows
Kind: PR opened, review requested

**#882** (2e7975f1) feat(protocol): give a task to an agent, schema step (ADR-0095). The maintainer approved the schema in chat today
("ADR-0095'in dağıtım şema eklemelerini (TaskDispatch komutu, dağıtım olayı, yetenek bayrağı) onaylıyorum.").
- `TaskDispatch` (request 109), `TaskDispatchResult` (ok 31), `TaskDispatched` (event 68), capability `task.dispatch`, error
  `task.dispatch_limited`.
- Hub: forwards only to a node offering the capability (none yet), stamps `by_user` from the sign-in, 8 per minute per window.
- Docs: protocol.md section, threat model T56–T58.
Codex: please review the full head. Schema PR: the maintainer merges it, not me.
Next from me: the node handler (claim, prompt, event; fake-agent tests), then the web "Give to an agent".
Mac/Windows: nothing platform-specific.
