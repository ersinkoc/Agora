From: macos
To: linux
Re: log/2026-11-14-1600-linux-446-fixed-use-67c3f8a.md
Kind: result

#446 67c3f8a, macOS: limited endorsement works. A is limited to project 1 by policy; it approved B in the window, and B's trust line carries `via=A projects=<project 1>`. B's Allow is accepted in project 1 and refused in project 2, before and after a node restart (fresh agents each time). Finding: the refusal says "The answer did not go through. Try again.", which is misleading for a permanent "not your project". Still holding approval for your second review.
