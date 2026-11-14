From: windows
To: macos
Re: log/2026-11-14-1325-macos-495-tui-colours.md
Kind: result

**#495 7878841 in real Windows terminals: Windows Terminal is good in every mode; conhost has two problems.**
Setup: debug build, scratch stack with a shell and a fake chat agent asking, 120×34. Each window was opened with NO_COLOR, TERM and COLORTERM cleared, as a user's would be. Only the window itself was captured.

| terminal | mode picked | result |
|---|---|---|
| Windows Terminal, Campbell (dark) | no TERM → 24-bit | good: "Needs you" bold orange, secondary text grey, dividers grey |
| Windows Terminal, One Half Light | no TERM → 24-bit | good: "Needs you" bold orange, readable on white |
| WT light, `TERM=xterm-256color` | 256 | good, the same as 24-bit to the eye |
| WT light, `TERM=xterm` | 16 | good: "Needs you" bold bright red; secondary text is black (no grey in 16), fine |
| conhost, default (dark) | 24-bit | colours right, but **the dividers are orange**, not grey |
| conhost, `color F0` (light) | 24-bit | **every run of text gets a black background**; empty cells stay white |

1. **Orange dividers in conhost:** conhost seems to draw the faint attribute (SGR 2) as dark yellow. In a run with NO_COLOR=1 set, conhost drew the dividers and the dimmed text orange, while Windows Terminal drew them grey. If the dividers and secondary text use DIM, an explicit grey (e.g. indexed 8 / DarkGray, or a 24-bit grey) would look the same in both.
2. **Black boxes in light conhost:** likely conhost's "default background" after the `color` command. Text cells seem to be written with an explicit black or reset background, which conhost resolves to its VT default (black) instead of the console's white. Windows Terminal light is fine with the same output. A real light conhost user sets colours in Properties, which I did not test (it changes the user's console settings). Lower priority than (1), but worth a look if cells are written with `Color::Black`/`Reset` as background.

Also seen: with NO_COLOR=1, conhost still showed coloured dividers (point 1), so NO_COLOR is not fully honoured there. Screenshots on request; I can attach them to the PR.
