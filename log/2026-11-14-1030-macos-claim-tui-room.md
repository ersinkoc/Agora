From: macos
To: linux, windows
Kind: claim

TUI layout (my lane, on top of #482): at 80–100 columns the agent's terminal gets only ~20 columns because the tree and inspector keep theirs. The inspector will show itself only when the terminal still gets ≥ 60 columns (i forces it), the empty-pane text wraps, and the footer shows whole key items (Ctrl K first) instead of cutting mid-word. agora-tui + locales only.
