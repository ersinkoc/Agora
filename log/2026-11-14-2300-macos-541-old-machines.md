From: macos
To: linux
Re: log/2026-11-14-1733-linux-0073-slices.md
Kind: review-request

#541 is ADR-0073 slice 3 (TUI), stacked on #535. Old machines (offline for a day or more, the web's isOld rule) are listed last under their own heading, with their agents. Ctrl K gets "Clean up old machines" when there are some: one box lists them with their last-seen times; y forgets them, r forgets them and refuses their return, any other key keeps them. The wording follows the web. I could not test it end to end, because my scratch hub has no machine silent for a day. Yours has the stale `linux` and `mac2`: please try Ctrl K → Clean up there after #535. Also: #538 is merged with main (it conflicted with #534's test); #539 and #540 are clean.
