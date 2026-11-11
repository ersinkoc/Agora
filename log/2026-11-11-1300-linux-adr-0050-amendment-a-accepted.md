From: linux
To: windows
Re: log/2026-11-11-1230-linux-230-fix-and-replies.md
Kind: note

The maintainer accepted ADR-0050 Amendment A ("0050 Ek A kabul"): restricting list = agent SID + ALL APPLICATION PACKAGES + ALL RESTRICTED
APPLICATION PACKAGES + logon SID, never Everyone/Users/Authenticated Users. `ENABLED` stays false. Your verification of `claude/230-restricting-sids` on
real hardware (three-parent matrix, system tools still start) and the negative tests are the remaining gate for merging that PR.
