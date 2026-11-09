# 0041. User-signed grants for sensitive commands routed through the hub
Status: Accepted: 2026-11-09 (maintainer's explicit approval in chat)
Date: 2026-11-09

## Context
ADR-0020 says a hub breach "cannot execute privileged actions on nodes": the node enforces policy, and sensitive actions need a
signature from a trusted device (ADR-0028). What the node enforces today is narrower: `approval_required` covers what *an agent asks to
do* (an approval request the user signs). The commands a client sends through the hub (ADR-0029) are not covered: `agent.start`,
`agent.prompt`, `agent.set_mode`, `terminal.open`, `terminal.input`, `files.write`, `worktree.*` run as soon as they reach the node. A
compromised hub, or a stolen client session (T2), can therefore start an agent and type into it, which is code execution on the machine.
The Codex review of 2026-11-09 found this (`docs/reviews/2026-11-09-codex-security-surfaces.md`, finding 1); the threat model (T1) now says it.

Signing every keystroke is not workable, and neither is a prompt on every `agent.prompt`. The signature has to buy a *grant* that covers
a stretch of use.

## Decision
1. **A grant, signed by a trusted device, authorises a class of commands for one scope and a short time.** The signed message is
   `agora-grant-v1 NUL node_id NUL grant_id NUL scope NUL commands NUL expires_at_ms` (the same ES256 scheme and trusted-devices file as
   ADR-0028). `scope` is `node` (all agents on the node), `agent:<id>` or `project:<id>`; `commands` is a list of command names; the
   expiry is at most 8 hours after issue (default 1 hour).
2. **The node stores grants in memory only** and checks one on every routed command that its policy lists in a new line of the policy
   file, `signed_commands = agent.start, agent.prompt, terminal.input, ...` (a prefix ending in `*` or `*` works as for
   `approval_required`). A command that is listed and has no live grant is refused with `approval.grant_required` and the name of the
   command; nothing runs. Grants are lost on restart on purpose.
3. **Defaults.** An absent `signed_commands` line means today's behaviour, so nothing changes for a machine until its owner sets the policy
   (a deliberate change, like the existing policy lines; the hub cannot change the file). The documented recommendation for a machine
   that is not a throwaway is `agent.start, terminal.input, files.write`, so reading (list, diff, history, chat, terminal output) stays free.
4. **Protocol (new messages and numbers only, ADR-0006).** `GrantCreate { grant_id, scope, commands[], expires_at_ms, signer_device_id,
   signature }` as a new routed command, answered with `Empty` or the error; `CommandRequest` is unchanged. A grant created through the
   hub is verified on the node, so a hub that forges one fails the signature check. `GrantList` / `GrantRevoke` let the owner see and end
   grants (revocation needs no signature: ending access is always allowed to a signed-in client).
5. **Local control socket is not affected.** A command that arrives on the node's own socket (the person at that machine, or an agent's
   own tools) is not routed through the hub and needs no grant.
6. **Web client.** The console asks for a grant when a command is refused with `approval.grant_required`, shows what it covers and for
   how long, signs it with the device key already used for approvals, and then repeats the command. No UI text names internal terms: "Allow
   typing into this agent for 1 hour".

## Security
- Closes T1 for the listed commands: a compromised hub cannot create a grant, so it cannot start or drive an agent on a node whose policy
  lists them. It can still deny service and read what the hub already sees (metadata, terminal output it relays).
- A stolen client session (T2) cannot sign a grant without the device key (passkey or non-extractable key), so it can read but not act.
- Replay: a grant is bound to the node id, a random grant id and an expiry; the node refuses a grant id it has seen and one already past its
  expiry. A grant for another node does not verify.
- The blast radius of a grant is its scope and time; `node` scope with all commands is allowed but the console defaults to one agent, one hour.

## Consequences
- Schema: `GrantCreate`, `GrantList`, `GrantRevoke` and their results (new message numbers), golden fixtures, error code
  `approval.grant_required`. `schema-breaking` stays green.
- Node: a grant store beside the approvals queue, a check in the routed-command path only, a policy line, tests (no grant refuses, a valid grant
  allows, wrong signer, wrong node, expired, replayed id, scope mismatch, local socket unaffected, hub-forged grant fails).
- Hub: routes the new commands like any other (ADR-0029); no new authority.
- Web: the grant prompt and the retry. Docs: protocol, threat model (T1 text), policy file format.
- A machine with the line set is less convenient: typing into an agent needs a grant first. That is the point; the default stays open until
  the owner chooses.

## Alternatives considered
- **Sign every sensitive command**: strongest, but unusable for `terminal.input` and slow for `agent.prompt`.
- **Make the hub a trusted party and rely on mTLS and sessions**: this is today's state and what the review flagged; a hub compromise is
  full compromise of every node.
- **Per-session proof-of-possession on the client connection** (sign a challenge when connecting): stops a stolen *token* but not a hub that
  speaks for the user, and gives the node nothing it can check for a routed command.
- **Make the line default to the recommendation**: safer by default, but it would break every existing install on upgrade; a later ADR can flip
  the default once the console flow exists.
