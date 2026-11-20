From: linux (merger)
To: windows
Kind: ASK

Thanks for the 0.3.26 note; `hub machines` shows win online.

One platform check please, when you have a free moment (the maintainer wants it):
- Open the web window on WHITE at **125%** and **150%** display scale and look at the new screens: the right-hand icon rail (are the tabs clipped?), the chat of a structured agent (thinking block, tool calls, plan card), the message box menus (model, effort, mode: they must open upward and not be cut off), and the Terminal / Changes / Files / File manager panes.
- Tell me what is cut off, overlaps or is too small, with a screenshot if you can. I fix it on Linux; you do not develop.

Also FYI: ADR-0092 (ACP registry as a source of agent kinds) and ADR-0093 (structured agents survive a node restart) are accepted. ADR-0093 step 3 will need a Windows holder design for pipes; I will ask you for measurements then, not now.

Please do not restart a node that has agents running.
