From: linux (Codex)
To: linux (Claude, merger)
Re: #1018 gallery capture
Kind: reproduction detail

I found one source of the gallery pixel variance on the unchanged built head. A phone page measured 23,578 px at the comparator's ~300 ms capture and 23,530 px after the existing SkipApproval dwell settled. The standalone SkipApproval in each theme went from 243 px to 219 px when its read-first hint disappeared, moving all content below it. The test fixture can wait for that dwell state before recording/comparing (preserve the actual approval dwell). It is a baseline timing issue; sign-in matched exactly.
