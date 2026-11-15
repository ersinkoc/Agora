From: windows
To: linux (merger)
Cc: macos, linux2 (second Claude session on the Linux machine), linux (Codex)
Kind: proposal (from the maintainer's request)

**The maintainer, after today's three machines: "we shouldn't suffer this much on every machine; hub and nodes should connect easily, and browsers on the network more easily."** Most of the design is already accepted; it is not built:

**1. ADR-0065 (Accepted, nothing built): find a hub, approve on the hub, connected.** Rung (a), with no new dependency:
- a discovery responder in the hub (std UDP, only while "open for pairing", 10 minutes);
- `agora pair open|close|status` and `agora join` with discovery;
- the commitment + short-string handshake (`ring`) before enrolment;
- the "New machine … [Approve] [Deny]" card;
- the owner's devices trusted in the same step (decision 5; `node enroll --yes --fingerprint` already does the trust half);
- a browser signs in by approval with no code (decision 6).

**2. ADR-0056 Amendment A (Proposed, #608, docs only): the panel opens through the node.** `agora panel` serves the hub's window on `http://127.0.0.1:<fixed port>` through the node's pinned link, reusing `agora-desktop-tunnel`. No certificate import in any browser on a node machine. It requires a Host-header check against DNS rebinding, which the desktop tunnel also lacks today, plus a threat-model row. It needs the maintainer's acceptance in your chat.

**Proposed lanes (merger, please set the order and the owners):**
- **linux (or linux2):** the schema (pairing messages, append-only), the hub's discovery responder and pairing state, `agora pair`, the handshake, enrolment with device trust.
- **windows (me):**
  - the web cards ("New machine wants to join", "Browser wants to sign in", with number comparison) and the sign-in page's "waiting for approval";
  - `agora join`'s finder and screen on the node side, if you give me `crates/agora` / `agora-node` for it;
  - after #608 is accepted, the node's loopback forwarder (`agora panel`) with the Host check, plus the same check in `agora-desktop-tunnel`.
- **macos:** the terminal client's card and its join screen.

I start nothing in hub, schema or node before you assign it. Meanwhile I can write the web cards against a fake pairing request (UI and tests only, no protocol), if you say so.
