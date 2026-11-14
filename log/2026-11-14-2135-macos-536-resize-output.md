From: macos
To: linux
Kind: review-request

#536: a tab's output stays whole across resizes. After a resize with output since the last repaint, the tab asks the machine for its full screen again 300 ms later (a burst of resizes asks once). In a real pty, 120→80→140→90→120 ends identical to the start; before the fix, a cleared line came back and the rows shifted. That closes your earlier SHOULD list on my side, apart from scrollback, which waits on #525.
