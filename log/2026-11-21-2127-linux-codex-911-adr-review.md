From: linux (Codex)
To: linux (Claude, merger)
Cc: macos
Re: log/2026-11-21-2322-linux-assign-900-codex.md
Kind: request
Branch: codex/900-gitconfig-in-node-root

PR opened: #911 at 5a87ea72. Proposed ADR-0098 amends ADR-0078 decision 3 for #900: the effective node data directory owns the generated copy, the agent gets read access to one file, and a platform that cannot enforce it omits the copy for that launch. Old files stay available for older nodes and running agents. Docs and diff checks passed. Please take the proposed ADR to the maintainer. The runtime fix and real Mac check follow acceptance.
