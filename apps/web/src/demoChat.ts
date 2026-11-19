import type { ChatLine } from './net/chat';

// A long, realistic conversation for the gallery and the demo window: two days, user requests, replies in Markdown (a table, a link,
// code blocks in three languages), runs of tool calls, several diffs of one file, a plan and a notice. Fixed times, so a screenshot is
// the same every time.
const day = (d: number, h: number, m: number, s = 0) => new Date(2026, 10, d, h, m, s).getTime();
const tool = (name: string, args: Record<string, unknown>) => JSON.stringify({ tool: name, arguments: args });

// ADR-0090: what a node with `chat.rich` sends: thinking, calls with a kind, status, duration, files and output, a plan with step status, a
// request for permission, a prompt waiting behind the turn. Same shape as the approved design: one turn, one login fix.
export function demoRichConversation(): ChatLine[] {
  const t0 = new Date(2026, 10, 19, 14, 2, 0).getTime();
  const call = (id: number, atMs: number, name: string, args: Record<string, unknown>, over: Partial<ChatLine>): ChatLine => ({ id, atMs, role: 'agent', kind: 'tool', text: tool(name, args), path: '', callId: `c${id}`, ...over });
  return [
    { id: 1, atMs: t0, role: 'user', kind: 'text', text: 'The login redirect sends people to /home after sign-in. It should go to the dashboard. Fix it and update the test.', path: '' },
    { id: 2, atMs: t0 + 1000, role: 'agent', kind: 'thought', text: 'The redirect target is probably hard-coded in the session handler. ', path: '' },
    { id: 3, atMs: t0 + 5000, role: 'agent', kind: 'thought', text: 'I should find where it is built before changing the test, so the test and the code agree on one source of truth.', path: '' },
    { id: 4, atMs: t0 + 10_000, role: 'agent', kind: 'text', text: "I'll look at where the redirect is decided, then change it and the test that pins it.", path: '' },
    call(5, t0 + 11_000, 'search', { pattern: 'redirect', path: 'src' }, { callKind: 'search', status: 'completed', startedMs: t0 + 11_000, endedMs: t0 + 11_600, output: 'src/session.rs:41     let target = "/home";\nsrc/session.rs:58     redirect(target)\ntests/login.rs:12    assert_eq!(res.location(), "/home");', locations: [{ path: 'src', line: 0 }] }),
    call(6, t0 + 12_000, 'read', { file_path: 'tests/login.rs' }, { callKind: 'read', status: 'completed', startedMs: t0 + 12_000, endedMs: t0 + 12_200, locations: [{ path: 'tests/login.rs', line: 12 }] }),
    call(7, t0 + 13_000, 'edit', { file_path: 'src/session.rs', old_string: '"/home"', new_string: '"/dashboard"' }, { callKind: 'edit', status: 'completed', startedMs: t0 + 13_000, endedMs: t0 + 13_300, locations: [{ path: 'src/session.rs', line: 41 }] }),
    { id: 8, atMs: t0 + 13_400, role: 'agent', kind: 'diff', text: '--- a/src/session.rs\n+++ b/src/session.rs\n@@ -39,7 +39,7 @@ fn after_sign_in\n     let user = session.user();\n-    let target = "/home";\n+    let target = "/dashboard";\n     log::info!("signed in {user}");\n', path: 'src/session.rs' },
    { id: 9, atMs: t0 + 14_000, role: 'agent', kind: 'plan', text: 'Find where the redirect is decided\nChange the target to /dashboard\nUpdate the test that pins the old target\nRun the whole suite', path: '',
      steps: [{ text: 'Find where the redirect is decided', status: 'completed' }, { text: 'Change the target to /dashboard', status: 'completed' }, { text: 'Update the test that pins the old target', status: 'in_progress' }, { text: 'Run the whole suite', status: 'pending' }] },
    { id: 10, atMs: t0 + 15_000, role: 'agent', kind: 'text', text: 'The target was a string in `after_sign_in`, so it was a one-line change. Now the test.', path: '' },
    call(11, t0 + 16_000, 'execute', { command: ['bash', '-lc', 'cargo test --no-fail-fast login'] }, { callKind: 'run', status: 'failed', startedMs: t0 + 16_000, endedMs: t0 + 21_000, output: 'running 2 tests\ntest redirects_to_dashboard ... ok\ntest redirect_keeps_return_url ... FAILED\n\nfailures:\n    redirect_keeps_return_url\n\ntest result: FAILED. 1 passed; 1 failed' }),
    call(12, t0 + 22_000, 'execute', { command: ['bash', '-lc', 'cargo test login'] }, { callKind: 'run', status: 'in_progress', startedMs: t0 + 22_000, output: '   Compiling agora v0.3.24\n    Finished test profile in 3.2s\n     Running tests/login.rs\ntest redirects_to_dashboard ... ok\ntest redirect_keeps_return_url ... running' }),
    { id: 13, atMs: t0 + 23_000, role: 'agent', kind: 'permission', text: 'run_command', path: '', approvalId: 'ap-1', outcome: 'pending' },
    { id: 14, atMs: t0 + 24_000, role: 'user', kind: 'text', text: 'Then update the changelog', path: '', queued: true },
  ];
}

export function demoConversation(): ChatLine[] {
  let id = 0;
  const out: ChatLine[] = [];
  const add = (atMs: number, role: ChatLine['role'], kind: ChatLine['kind'], text: string, path = '', extra: Partial<ChatLine> = {}) => { out.push({ id: ++id, atMs, role, kind, text, path, ...extra }); };
  const user = (atMs: number, text: string) => add(atMs, 'user', 'text', text);
  // The adapter sends a reply in chunks; the window shows them as one reply.
  const say = (atMs: number, text: string) => { const mid = Math.floor(text.length / 2); add(atMs, 'agent', 'text', text.slice(0, mid)); add(atMs + 150, 'agent', 'text', text.slice(mid)); };
  const steps = (atMs: number, list: [string, Record<string, unknown>][]) => list.forEach(([n, a], i) => add(atMs + i * 2500, 'agent', 'tool', tool(n, a)));

  user(day(18, 16, 2), 'The login test fails on CI but passes locally. Can you find out why?');
  add(day(18, 16, 2, 5), 'agent', 'plan', 'Run the login test\nFind why it fails on CI\nFix it and run the suite');
  steps(day(18, 16, 2, 8), [
    ['execute', { command: ['bash', '-lc', 'cargo test -p auth login'] }],
    ['Read src/auth.rs', { file_path: 'src/auth.rs', limit: 200 }],
    ['grep "redirect"', { pattern: 'redirect', path: 'tests' }],
    ['Read tests/login.rs', { file_path: 'tests/login.rs' }],
    ['Read .github/workflows/ci.yml', { file_path: '.github/workflows/ci.yml' }],
  ]);
  say(day(18, 16, 3), 'Found it. CI runs with `APP_ENV=ci`, and in that mode the redirect after login goes to `/dashboard`, not `/home`. The test hard-codes the old path, so it only passes when the variable is unset.\n\n| Where | Redirect | Source |\n|---|---|---|\n| local | `/home` | `src/auth.rs:41` |\n| CI | `/dashboard` | `src/auth.rs:58` |\n\nI will make the test read the same constant as the code. See the [redirect notes](https://example.com/docs/redirects) for the background.');
  steps(day(18, 16, 4), [['Edit `tests/login.rs`', { file_path: 'tests/login.rs', old_string: '"/home"', new_string: 'auth::LOGIN_REDIRECT' }]]);
  add(day(18, 16, 4, 6), 'agent', 'diff', '--- a/tests/login.rs\n+++ b/tests/login.rs\n@@ -12,7 +12,7 @@\n     let res = login(&client, "ada", "pw").await;\n-    assert_eq!(res.location(), "/home");\n+    assert_eq!(res.location(), auth::LOGIN_REDIRECT);\n', 'tests/login.rs');
  steps(day(18, 16, 5), [['execute', { command: ['bash', '-lc', 'APP_ENV=ci cargo test -p auth'] }]]);
  say(day(18, 16, 6), 'The suite passes in both modes now:\n\n```sh\nAPP_ENV=ci cargo test -p auth\n```\n\nShall I also add a regression test for the CI mode?');
  user(day(18, 16, 9), 'Yes, please. Keep it small.');
  steps(day(18, 16, 10), [
    ['Read src/config.rs', { file_path: 'src/config.rs' }],
    ['Write `tests/login_ci.rs`', { file_path: 'tests/login_ci.rs', content: '...' }],
    ['Edit `src/auth.rs`', { file_path: 'src/auth.rs', old_string: 'pub const LOGIN_REDIRECT', new_string: 'pub const LOGIN_REDIRECT' }],
  ]);
  add(day(18, 16, 10, 20), 'agent', 'diff', '--- /dev/null\n+++ b/tests/login_ci.rs\n@@ -0,0 +1,9 @@\n+use auth::LOGIN_REDIRECT;\n+\n+#[tokio::test]\n+async fn ci_mode_redirects_to_the_dashboard() {\n+    std::env::set_var("APP_ENV", "ci");\n+    let res = login(&client(), "ada", "pw").await;\n+    assert_eq!(res.location(), LOGIN_REDIRECT);\n+}\n', 'tests/login_ci.rs');
  add(day(18, 16, 10, 24), 'agent', 'diff', '--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -55,3 +55,4 @@\n-pub const LOGIN_REDIRECT: &str = "/home";\n+/// Where a successful login goes; CI sets `APP_ENV=ci` to land on the dashboard.\n+pub const LOGIN_REDIRECT: &str = "/dashboard";\n', 'src/auth.rs');
  say(day(18, 16, 12), 'Added `tests/login_ci.rs`:\n\n```rust\n#[tokio::test]\nasync fn ci_mode_redirects_to_the_dashboard() {\n    std::env::set_var("APP_ENV", "ci");\n    assert_eq!(login().await.location(), LOGIN_REDIRECT);\n}\n```\n\n- the test runs in **both** modes\n- the constant now carries a doc comment\n- no other file changed');

  user(day(19, 9, 30), 'Good morning. Can you also move the retry helper out of `auth.rs`? It is used in three places now.');
  add(day(19, 9, 30, 6), 'agent', 'plan', 'Run the login test\nFind why it fails on CI\nFix it and run the suite\nMove the retry helper to its own module\nUpdate the three call sites');
  steps(day(19, 9, 31), [
    ['grep "retry"', { pattern: 'retry', path: 'src' }],
    ['Read src/auth.rs', { file_path: 'src/auth.rs' }],
    ['Read src/api.rs', { file_path: 'src/api.rs' }],
    ['Read src/session.rs', { file_path: 'src/session.rs' }],
    ['Write `src/retry.rs`', { file_path: 'src/retry.rs', content: '...' }],
    ['Edit `src/auth.rs`', { file_path: 'src/auth.rs', old_string: 'fn retry', new_string: '' }],
    ['Edit `src/api.rs`', { file_path: 'src/api.rs', old_string: 'auth::retry', new_string: 'retry::with_backoff' }],
    ['Edit `src/session.rs`', { file_path: 'src/session.rs', old_string: 'auth::retry', new_string: 'retry::with_backoff' }],
    ['execute', { command: ['bash', '-lc', 'cargo test --workspace'] }],
  ]);
  add(day(19, 9, 33), 'agent', 'diff', '--- /dev/null\n+++ b/src/retry.rs\n@@ -0,0 +1,14 @@\n+use std::time::Duration;\n+\n+/// Runs `f` until it succeeds or `tries` runs out, waiting longer each time.\n+pub async fn with_backoff<T, E, F, Fut>(tries: u32, mut f: F) -> Result<T, E>\n+where\n+    F: FnMut() -> Fut,\n+    Fut: std::future::Future<Output = Result<T, E>>,\n+{\n+    let mut wait = Duration::from_millis(100);\n+    for _ in 1..tries {\n+        if let Ok(v) = f().await { return Ok(v); }\n+        tokio::time::sleep(wait).await;\n+        wait *= 2;\n+    }\n+    f().await\n+}\n', 'src/retry.rs');
  add(day(19, 9, 33, 3), 'agent', 'diff', '--- a/src/api.rs\n+++ b/src/api.rs\n@@ -3,5 +3,5 @@\n-use crate::auth::retry;\n+use crate::retry::with_backoff;\n', 'src/api.rs');
  say(day(19, 9, 36), '## Done\n\nThe helper lives in `src/retry.rs` as `with_backoff`, and the three call sites use it. All tests pass.\n\nA few things worth a look:\n\n1. `with_backoff` doubles the wait each time, starting at 100 ms\n2. it gives up after `tries` attempts and returns the last error\n3. `<script>alert(1)</script>` in a comment is shown as text, never run\n\n```typescript\nconst wait = (ms: number) => new Promise((r) => setTimeout(r, ms));\n```');
  add(day(19, 9, 37), 'agent', 'notice', '', '', { code: 'adapter.turn_empty', params: {} });
  user(day(19, 9, 40), 'Thanks. Run the whole suite once more and tell me how long it takes.');
  steps(day(19, 9, 41), [['execute', { command: ['bash', '-lc', 'time cargo test --workspace'] }]]);
  say(day(19, 9, 44), 'The whole workspace takes about 41 seconds: 38 s to build, 3 s to run 212 tests. Nothing failed.');
  return out;
}
