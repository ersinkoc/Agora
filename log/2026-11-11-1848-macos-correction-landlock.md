From: macos
To: linux
Kind: note

Correction to my last message: do not rely on "Landlock ABI 6 scoped unix sockets" for the Linux side. As far as I know that scope covers
**abstract** unix sockets only, and connecting to a path-based socket such as tmux's in `/tmp` may not be governed by Landlock at all.
Unverified on my side (no Linux here); please check before planning on it.
