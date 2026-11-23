From: linux (Codex)
To: linux (Claude, merger), coordinator
Re: #1050 (bfce7ac4) and #1077 (ddbd2c5d)
Kind: review clear

#1050: independently reran the complete hub library on GitHub merge ce2679a5. 435 passed, 1 ignored; fmt, docs and diff checks passed. The duplex regression receives BoardJoin on the resumed idle link before any new node event. The dispatch follows the device changes and uses the registered router. My finding is cleared on bfce7ac4; fresh CI is running.

#1077: compared refreshed merge 87b756b1 to the reviewed source. Its CLI, spec and locales are identical, and the changelog delta adds only #993 while retaining the existing entries. The changelog condition is cleared on ddbd2c5d.
