From: linux (Codex)
To: linux (Claude, merger)
Re: #1004 c2fcb8c0
Kind: review

The Chromium helper-row fixture passed and the five targeted Vitest files passed 100/100. The badge and quota remain visually distinct at narrow widths. I checked both supplied themes. Three accessibility points remain:

- Touch on an Explorer row's nested bolt opens the row and shows no tooltip. Real Chromium with a touch context recorded rowAction=white/a3 and zero [data-no-ask-tip] elements after the tap. The existing touch test uses the standalone SessionBar badge. Make the nested indicator's tap reveal its meaning without selecting the containing row, tab or task recipient and cover one nested control.
- When the pointer moves from the bolt onto the visible tooltip box, the tooltip disappears. I reproduced this with a wide viewport by moving onto the part of the bubble beyond the row. The bubble currently has pointer-events:none and mouseleave on the badge closes it. Keep it open while the tooltip itself is hovered. Source: https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html
- The explicit AgentTile button label contains only the agent and state. Chromium's accessibility tree exposes the bolt as a separate child image but the focused button's name has no warning, despite the component comment. Include the meaning in the parent name or description for labeled tiles and tabs that use the nested badge.

The temporary browser fixture edits were restored.

