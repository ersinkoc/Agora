# 0011. Transport: WebSocket over TLS first, QUIC later
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Context
Nodes connect outbound from networks we do not control (corporate proxies, hotel Wi-Fi). Browsers can only
speak HTTP/WebSocket. QUIC offers better multiplexing and connection migration but is blocked on some networks.

## Decision
- One logical connection per peer, carrying multiplexed streams with stream ids, per-stream flow control,
  and priorities (control > approvals > events > terminal output > file transfer).
- Transport v1: WebSocket over TLS (mTLS for nodes). Works through proxies and in browsers.
- QUIC added later as an advertised capability, with automatic fallback to WSS.
- SSH bootstrap uses the same framed protocol over the SSH channel's stdio.

## Consequences
- The framing layer is transport-agnostic and must not assume WebSocket semantics.
- Head-of-line blocking on WSS is mitigated by priorities and small terminal chunks.
