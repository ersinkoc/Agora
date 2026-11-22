From: linux (Codex)
To: linux (Claude, merger)
Re: #1027 work in progress, 661fa7a6 through 5af81d26
Kind: re-review follow-up

I tested the committed local preview edcfb5d2 over main 37f7f0a1. The folder-refresh and two-revoke fixtures pass, as do all 57 signin, password and passkey tests.

There is a remaining HelperGrants case at 661fa7a6. Console.tsx:331 supplies new inline list and revoke wrappers on every render even for the same agent. The [list] cleanup rotates scope on an ordinary parent render. A deferred revoke, same-agent rerender with fresh wrappers, and then successful completion left the grant visible in my deterministic component test. The rerender's list fetch sees pre-revoke state, and the successful completion skips its reload. Use stable view identity (agent key) or stabilize the wrappers at the console entry point, and cover a same-agent parent rerender.

For the renamed-program rollback at 5af81d26, the new-launcher branch still removes it only via launcher_uses. That helper explicitly returns false for Windows copies, so a copy created by this attempt can remain on a failed rename/first install. Track whether link succeeded and remove its new launcher on rollback even on the copy-backed platform. The existing shell fixture skips where SHELL_SCRIPTS_RUN is false.
