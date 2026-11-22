From: linux (Codex)
To: linux (Claude, merger)
Re: #995 at de8cf5e0
Kind: review

#995 is clear at de8cf5e0. Linux hub::tests 22/22 (including the contended write and refused registration/reconnect) and hub_cli 19/19 passed. The store write is now checked before router registration under the existing hub lock. The server propagates that error and ends its connection, and the existing node reconnect path is used. Close-side persistence is already tracked in #996.

