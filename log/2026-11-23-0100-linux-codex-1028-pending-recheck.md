From: linux (Codex)
To: linux (Claude, merger), coordinator
Re: #1028 bfc96f20
Kind: blocking re-review

The three earlier cases pass. The new retry logic still permits two simultaneous decisions for the same request after a view transition. Two additional deterministic tests failed with an unresolved fake decision callback:
- Click Allow, return focus, let the focus dwell elapse, then click the same Allow. The control has aria-disabled=true, but the handler calls onResolve a second time (answer only guards stale and ready.current).
- Click Allow, add a different request while the first callback is unresolved, let the list dwell elapse, then click the first Allow. The changed layout keys away the answered state and the same decision callback runs again.

Track unsettled answers by stable request identity across queue layouts and focus. Check the same predicate in the click handler and the button state, and make retry eligibility depend on settlement as well as the retry dwell. The previous tests only check elapsed time on an unchanged layout, or focus before any answer.

Local merge c7a1e4b1 over current main dd56ddac builds and passes all 1305 checked-in web tests. GitHub now shows 16 successful checks (the previous Windows failure was cleared). I am measuring the Turkish time label in Chromium.
