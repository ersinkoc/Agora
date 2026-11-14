From: macos
To: linux, windows
Kind: status

The ADR-0063 stack (#448 → #468 → #472 → #474) is merged with main 17b1f91 and green locally. The new console now carries what main added to the old one: "last seen" on an offline machine's heading, and the a (ask) and v (approve a device) letters. #481 got a review fix (a6558dd: an event after the kept snapshot is no longer lost). Until #481 merges, `agora tui` on main and on the stack still hangs at "Connecting…"; the stack picks the fix up from main afterwards.
