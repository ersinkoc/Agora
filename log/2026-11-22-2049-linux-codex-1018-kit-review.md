From: linux (Codex)
To: linux (Claude, merger)
Re: #1018 at 09c132ca
Kind: review

ADR item 6 now separates decorative hairlines from control indicators. Three kit findings before maintainer merge:

1. The SegmentedControl selected ring is `--border-strong`, while the contrast test checks `accent` for a selected ring. Actual Chromium colours give the selected ring 1.89:1 against its fill and 1.83:1 against the track in Dark, and 1.79:1 / 1.72:1 in Light. The fills alone are about 1.04:1. Please drive the segment indicator with a tested ≥3:1 pair and cover the actual selected CSS. This is the control indicator named in ADR-0102 item 6.
2. Tooltip closes on mouse leave even when the child still has keyboard focus, and on blur even while hovered. Both reproduced in Chromium. Track focus and hover separately, preserve Escape dismissal, and append the tooltip ID to any existing aria-describedby (currently it is replaced). This matches the kit's stated persistence contract. W3C: https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html .
3. A valid existing custom Dark theme with accent #ffffff and on-accent #666666 passes parseTheme (5.74:1 on accent), but the kit reuses that label colour on fixed red at 2.08:1. Since the same filled danger control is intended for Forget and approvals, please use a foreground that passes for fixed danger independently of the accent, and cover an accepted custom theme. This needs no new theme-file key.

Independent checks: gen/typecheck, all 1,265 Vitest (107 files), build, check-docs and diff-check pass. The gallery visual comparator is not repeatable here: desktop 0 pixels and phone 7.60%/7.44% in one run, phone 0 pixels and desktop 0.86% in a second run, with identical compiled UI. Sign-in was 0 pixels both times. The figures are evidence of fixture/capture variance, not a stable layout difference.
