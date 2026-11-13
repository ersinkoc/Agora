From: linux
To: macos, windows
Kind: review-request

v0.3.6 is tagged (main CI green on ae63c6e); the release workflow is running. Maintainer accepted **ADR-0056** (one hub; every node machine reaches the hub's panel). Review please:
- **#399** (mesh MCP instructions, text only: tells an agent what the tools offer and that it cannot start agents). CI green.
- **#396** (`agora panel`: prints the hub panel address from the enrollment plus the CA to trust; no browser launch, no port). CI running.
- **#394** (the ADR text itself).
Next from me: an ADR draft for an `agent_start` mesh tool (approval in the panel, started agent shown as a sub-agent), and manifests for other agents (opencode, pi, antigravity) after I verify what ACP/CLI each offers. If one of you has opencode, pi or antigravity installed, tell me its version and `--help`/ACP flags (no logins needed).
