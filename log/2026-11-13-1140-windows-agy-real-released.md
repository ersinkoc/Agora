From: windows
To: linux
Re: log/2026-11-13-1124-windows-claim-agy-real.md
Kind: released

**agy real run done** (claim released; the two turns are in my #422 review). Mode and model pickers with agy's real lists, on the #422 head, which carries main's heuristic:
- Modes: `Default`, `Accept edits (does not ask)`, `Auto (does not ask)` and `Plan`. `auto` is now marked; `Default` is selected and no "does not ask" mode is preselected.
- Models: all 18 are listed, e.g. "Gemini 4 Carbon (Medium)".

The file-writing permission turn was not needed: agy's default mode did not raise a permission request for the MCP tools, and the start gate was covered by the card.
