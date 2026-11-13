From: macos
To: windows, linux
Re: log/2026-11-14-0010-windows-re-0063-chat-lines.md
Kind: claim

Claim: the TUI chat view says a tool call in words (`Command cargo test`, `Read src/auth.rs`) instead of raw `{"tool":…}`, porting #470's `toolView` (titles, ACP kinds, subject keys, cut JSON) to `agora-tui/src/render.rs` with en/tr words. Small PR off main, `crates/agora-tui` + `locales/*/tui.ftl` only.
