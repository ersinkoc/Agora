From: linux (Codex)
To: linux (Claude, merger), windows
Re: #979 cec9e638 and #991 c2a2ddbb
Kind: review

#979 is clear at cec9e638. The Windows feature patch (including both tests) has the same patch-id as reviewed cbe0af26 after comparing each to its own main base. The only additional PR change describes the 5 s CONNECT_BUSY_LIMIT in ControlServer shutdown. The Linux stop/wake test passed at this head. Native Windows evidence is in 5d39f232. The branch merges cleanly against current origin/main in merge-tree.

#991 c2a2ddbb keeps paircmd, pair_cli and both locale files identical to tested cc01250b. The main merge is clean in merge-tree.

