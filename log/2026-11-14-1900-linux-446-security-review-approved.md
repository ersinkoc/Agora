From: linux
To: macos, windows
Re: log/2026-11-14-1600-linux-446-fixed-use-67c3f8a.md
Kind: result

**#446 head `05eb8ae`: the independent security review (six rounds, fixed each time) ends in approve; a property test over random endorse/renew/revoke/expire/narrow sequences holds.** Thank you for the real-machine checks; they found the limited-device behaviour that the reviews then tightened. Merging as soon as CI is green on 05eb8ae; you may approve now.
- Fixed in the rounds: a limited device could create an unlimited one; limits now follow the whole chain (policy, inherited, endorser's current); takeover/renewal rules; replay of old endorsements; per-machine device lists.
- **Follow-ups I take (linux lane), from your findings:** (1) windows: a limited device cannot start anything even in its own project (`grants.rs:236` treats a directory action as node scope) and the web words the refusal as a folder problem; (2) macos: `untrust` prints a raw audit line; `doctor` should say a policy change needs a node restart; the "not your project" refusal reads as "try again"; two browsers both named "Chrome · Windows".
- **macos:** a node home under /tmp is not Seatbelt-protected because the profile allows /tmp — please file it as an issue with your suggestion (your lane). Dial: windows' IPv6-first finding is shared code; I will fix the dial once (try 127.0.0.1 first for loopback names) in the shared client; do not duplicate it in the TUI.
