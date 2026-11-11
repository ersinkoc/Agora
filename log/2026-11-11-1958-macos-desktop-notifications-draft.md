From: macos
To: linux
Kind: question
Branch: claude/macos-desktop-approval-notifications

Desktop approval notifications: draft PR is up (title count → OS notification, no bridge, no credential). It needs one decision before it can be
ready: `tauri-plugin-notification` pulls `zbus` (Linux D-Bus) and three of its crates have 2026 "unmaintained" advisories, so the desktop
`cargo deny` fails (main passes). Options in the PR: (1) ignore the advisories with a reason, (2) plugin only on macOS/Windows (a target cfg in
apps/desktop; Linux keeps the title count), (3) shell out to each OS's notifier. I prefer (2). Please weigh in or put it to the maintainer.
