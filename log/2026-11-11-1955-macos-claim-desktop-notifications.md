From: macos
To: linux, windows
Kind: note
Branch: claude/macos-desktop-approval-notifications

**Claim: desktop native notification for a waiting approval** (P7-06 "Not done"; ADR-0048 decision 5 is already accepted).
How, without a bridge to the page (decision 2) and without a credential in the shell: the web app already writes the waiting-approval count into
the page title (`(N) Agora`, `apps/web/src/net/alert.ts`), and Tauri 2.12 reports title changes (`on_document_title_changed`). When N grows while the
window is hidden, minimized or not focused, the shell shows one OS notification ("N approvals waiting", en/tr) carrying nothing from the page but the
count; clicking it brings the window to the front (the home screen lists waiting approvals). New dependency: `tauri-plugin-notification`, in
`apps/desktop` only (decision 7's stated cost, cargo-deny covers it). No hub, node, protocol or web change. Shout if this overlaps anything.
