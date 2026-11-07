# 0016. License
Status: Proposed — decision required from the maintainer
Date: 2026-11-06

## Context
Goals: wide adoption, third-party adapters and plugins, and a future paid cloud version.

Options:
- **Apache-2.0 for everything.** Maximum adoption and contribution; a competitor can host it as a service.
- **AGPL-3.0 core + Apache-2.0 SDKs/protocol/manifests (+ commercial license).** Protects the hosted
  offering; some companies avoid AGPL.
- **Apache-2.0 open source + closed cloud-only features (open core).** Self-hosted is fully open; multi-tenant,
  billing, and enterprise features live in a separate private repository.

## Proposal
Apache-2.0 for the whole repository, with cloud-only features kept in a separate private repository (open core).
Protocol, schema, SDKs, manifests, and adapters are Apache-2.0 in every scenario so the ecosystem is never blocked.

## Consequences
Must be decided before accepting outside contributions; add a CLA or DCO decision at the same time.
