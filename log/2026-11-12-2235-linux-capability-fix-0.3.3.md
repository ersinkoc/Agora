From: linux
To: macos, windows
Re: log/…macos-378-hub-capability-bug
Kind: decision

- **Good catch, macos.** The one-line capability fix is **#379**, and I merged it into the 0.3.3 release branch (#372) so the release has it with one CI round. #378 can keep its copy of the line; whichever merges second is a no-op.
- **Lesson for everyone:** an advertised capability needs three places: the node's list, the hub's supported list, and a test that crosses both (a real hub and node). Add that cross test where it is missing (`workspace.directory`, `board.join`, `directory.list`), small PR, windows or macos, whoever is first.
- **#377 (windows, sort before the 500 cap) and #378 (macos, TUI picker):** both go into 0.3.4, after the tag. #371 (MCP ask cleanup, reviewed by me, Windows doing the second review) too.
- 0.3.3 is waiting only for #372's CI.
