# PR #296: desktop approval notification review

Reviewed head: `f2c002504309111e635b9f6fb7a337ac91c1b7e8`.
Verdict: **CHANGES NEEDED** for the Linux notification activation path.

## P2: clicking a Linux notice has no path back to the desktop window

ADR-0048 decision 5 requires notification activation to open the approval page. `main.rs::notify_waiting` states that clicking brings the app forward by system default. Linux `desktop_notify` actually invokes `notify-send --app-name=Agora -- TITLE BODY`, discards output and returns once posting finishes. It registers no notification action, supplies no application desktop entry, and has no callback to restore/focus/navigate the Tauri window. The application name labels the notice; it does not implement activation.

The real compiled platform function was invoked against the installed `notify-send` 0.8.8, on a private session bus whose daemon advertises action support. The captured `Notify` call has `actions: []`, with only urgency and sender-PID hints; posting returns `Ok(())`. No notice reaches the user's desktop. The regression fails because no activation action exists. A positive control using a byte-identical local copy of the installed notifier with `--action=default=Open` records `["default", "Open"]` and returns `default` after the daemon emits `ActionInvoked` and closes the notification. The private daemon is capable of handling actions.

Control caveat: this host's AppArmor profile for `/usr/bin/notify-send` denies `GetCapabilities` to the private daemon. The control therefore uses the identical executable copied inside this worktree's ignored target directory. No installed binary or system security setting is edited. The production-path capture still uses the actual `/usr/bin/notify-send`; no action was requested there, independently of that profile. This proves the current request and missing handler, not GUI behavior under a real compositor.

The [freedesktop protocol](https://specifications.freedesktop.org/notification/latest/protocol.html) defines action identifiers on `Notify` and `ActionInvoked` for user activation. A display name is not a return route to this process.

Wire activation back to the existing window and the waiting approval view, on servers that support actions, instead of relying on the system to infer it. Test the activation path as well as notification posting. The equivalent Windows limitation was [already reported by its native reviewer](https://github.com/ersinkoc/Agora/pull/296#issuecomment-6895289857); this review adds Linux transport evidence rather than treating the Windows limitation as a new finding. If activation is deliberately deferred as a separate increment, document that scope accurately and leave ADR-0048 decision 5 incomplete; the current default-activation claim must not ship as implemented behavior.

## Verification

- `cargo test -p agora-platform -p agora-i18n`: **76 passed**, none failed/ignored.
- Root and desktop formatting, docs checker, and platform/i18n all-target Clippy with warnings denied pass.
- `0296-title-control.py` extracts the unchanged `waiting` function and callback body from `main.rs`, replacing only the Tauri window/notifier boundary with mocks: 10 inputs cover foreground, hidden, minimized, unfocused, repeated/decreased counts, absent/malformed/overflow prefixes and query-error defaults; expected notices are `[2,3,4]`, with 10/10 title forwards. The real i18n catalog renders all four en/tr singular/plural cases correctly. This is not a real Tauri/webview event test.
- `0296-argv-control.py` calls the real platform function against a private executable shim in a path containing spaces and Unicode. It verifies literal title/body arguments with an option-like title, shell metacharacters and a newline; the `--` delimiter; nonzero notifier status; nonexecutable notifier; relative PATH; missing PATH. Five calls pass. Environment changes are subprocess-local. No real notification or injected command is executed.
- Local full desktop tests cannot build: system `pkg-config`/D-Bus development libraries are missing. This is an environment limitation, not attributed to this PR. Reported desktop CI shell builds/Clippy pass on all three OSes, but that workflow does not run the desktop unit test or a GUI activation check.
- Native Windows toast/safety observations are attributed to the [Windows reviewer](https://github.com/ersinkoc/Agora/pull/296#issuecomment-6895289857), whose machine had notifications disabled. Visible Windows toast activation is unverified. macOS intentionally returns `Unsupported` in this increment.
- Platform arguments remain data, remote-page capabilities are unchanged, and the new locale command is invoked only by the shell's local page. No new third-party dependency is introduced by the implementation.

Nonblocking cleanup: the PR description still describes a notification-plugin dependency decision and an old draft state; rewrite it around the final no-plugin implementation and remaining activation limits. Linux's hardcoded `--app-name=Agora` should use the already localized title rather than introducing another product-name literal outside `locales/`.

## Reproduction

Run from an isolated checkout containing these evidence files:

```sh
cargo test -p agora-platform -p agora-i18n
python3 docs/reviews/0296-run-controls.py
```

The runner needs the system Python's `dbus` and `gi`, `dbus-run-session`, and `/usr/bin/notify-send`. Its native-notification regression is expected to fail (exit 1) on the reviewed head; the action-capable positive control, title/locale controls, and argument controls pass. No implementation or existing test files are modified.
