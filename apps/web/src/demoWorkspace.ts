import type { WorkspaceFiles } from './net/files';

// Demo data for the review screen and the file browser in the demo window (`#/demo`, development): the login-fix workspace of the
// closed demo agent "codex · rose", with what it changed. Produced by real git, so the hunks and line numbers are what a node sends.
const DIFF = "diff --git a/src/auth.rs b/src/auth.rs\nindex 686b7e1..efa2a4b 100644\n--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -1,4 +1,4 @@\n-use std::time::{Duration, SystemTime};\n+use std::time::Duration;\n \n use hmac::{Hmac, Mac};\n use sha2::Sha256;\n@@ -6,12 +6,14 @@ use sha2::Sha256;\n use crate::session::Session;\n \n /// How long a sign-in token stays valid.\n-pub const TOKEN_LIFETIME: Duration = Duration::from_secs(3600);\n+pub const TOKEN_LIFETIME: Duration = Duration::from_secs(15 * 60);\n \n #[derive(Debug, PartialEq, Eq)]\n pub enum AuthError {\n     Expired,\n     BadSignature,\n+    /// Issued after now: a clock that is off, or a forged time.\n+    FromTheFuture,\n }\n \n pub struct Token {\n@@ -30,8 +32,10 @@ fn mac(key: &[u8], user: &str, issued_secs: u64) -> Hmac<Sha256> {\n \n /// Checks a token and opens a session for its user.\n pub fn sign_in(token: &Token, key: &[u8], now_secs: u64) -> Result<Session, AuthError> {\n-    let age = now_secs.saturating_sub(token.issued_secs);\n-    if age > TOKEN_LIFETIME.as_secs() {\n+    if token.issued_secs > now_secs {\n+        return Err(AuthError::FromTheFuture);\n+    }\n+    if now_secs - token.issued_secs > TOKEN_LIFETIME.as_secs() {\n         return Err(AuthError::Expired);\n     }\n     mac(key, &token.user, token.issued_secs).verify_slice(&token.tag).map_err(|_| AuthError::BadSignature)?;\ndiff --git a/src/sessions.rs b/src/session.rs\nsimilarity index 100%\nrename from src/sessions.rs\nrename to src/session.rs\ndiff --git a/tests/auth_test.rs b/tests/auth_test.rs\nnew file mode 100644\nindex 0000000..60cb55f\n--- /dev/null\n+++ b/tests/auth_test.rs\n@@ -0,0 +1,7 @@\n+use login_fix::auth::{sign_in, AuthError, Token};\n+\n+#[test]\n+fn a_token_from_the_future_is_refused() {\n+    let token = Token { user: \"ada\".into(), issued_secs: 2_000, tag: vec![0; 32] };\n+    assert_eq!(sign_in(&token, b\"key\", 1_000).err(), Some(AuthError::FromTheFuture));\n+}\n";

const FILES: Record<string, string> = {
  "src/auth.rs": "use std::time::Duration;\n\nuse hmac::{Hmac, Mac};\nuse sha2::Sha256;\n\nuse crate::session::Session;\n\n/// How long a sign-in token stays valid.\npub const TOKEN_LIFETIME: Duration = Duration::from_secs(15 * 60);\n\n#[derive(Debug, PartialEq, Eq)]\npub enum AuthError {\n    Expired,\n    BadSignature,\n    /// Issued after now: a clock that is off, or a forged time.\n    FromTheFuture,\n}\n\npub struct Token {\n    pub user: String,\n    pub issued_secs: u64,\n    pub tag: Vec<u8>,\n}\n\n/// The tag binds the user and the time of issue, under the server's key.\nfn mac(key: &[u8], user: &str, issued_secs: u64) -> Hmac<Sha256> {\n    let mut m = Hmac::<Sha256>::new_from_slice(key).expect(\"HMAC takes a key of any length\");\n    m.update(user.as_bytes());\n    m.update(&issued_secs.to_be_bytes());\n    m\n}\n\n/// Checks a token and opens a session for its user.\npub fn sign_in(token: &Token, key: &[u8], now_secs: u64) -> Result<Session, AuthError> {\n    if token.issued_secs > now_secs {\n        return Err(AuthError::FromTheFuture);\n    }\n    if now_secs - token.issued_secs > TOKEN_LIFETIME.as_secs() {\n        return Err(AuthError::Expired);\n    }\n    mac(key, &token.user, token.issued_secs).verify_slice(&token.tag).map_err(|_| AuthError::BadSignature)?;\n    Ok(Session::open(&token.user))\n}\n",
  "src/session.rs": "pub struct Session {\n    pub user: String,\n}\n\nimpl Session {\n    pub fn open(user: &str) -> Self {\n        Self { user: user.to_string() }\n    }\n}\n",
  "tests/auth_test.rs": "use login_fix::auth::{sign_in, AuthError, Token};\n\n#[test]\nfn a_token_from_the_future_is_refused() {\n    let token = Token { user: \"ada\".into(), issued_secs: 2_000, tag: vec![0; 32] };\n    assert_eq!(sign_in(&token, b\"key\", 1_000).err(), Some(AuthError::FromTheFuture));\n}\n",
  "src/lib.rs": "pub mod auth;\npub mod session;\n",
  "Cargo.toml": "[package]\nname = \"login-fix\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nhmac = \"0.12\"\nsha2 = \"0.10\"\n",
  "README.md": "# login-fix\n\nSign-in tokens with a short lifetime, bound to their user and time of issue by an HMAC.\n"
};

export async function demoDiff(path = ''): Promise<string> {
  if (!path) return DIFF;
  const at = DIFF.indexOf(`diff --git a/${path} `);
  if (at < 0) return '';
  const next = DIFF.indexOf('\ndiff --git ', at + 1);
  return DIFF.slice(at, next < 0 ? undefined : next + 1);
}

export const demoFiles: WorkspaceFiles = {
  list: async (dir) => {
    const prefix = dir ? `${dir}/` : '';
    const names = new Map<string, { isDir: boolean; size: number }>();
    for (const [p, text] of Object.entries(FILES)) {
      if (!p.startsWith(prefix)) continue;
      const rest = p.slice(prefix.length);
      const cut = rest.indexOf('/');
      if (cut >= 0) names.set(rest.slice(0, cut), { isDir: true, size: 0 });
      else names.set(rest, { isDir: false, size: new TextEncoder().encode(text).length });
    }
    return [...names.entries()].map(([name, v]) => ({ name, ...v })).sort((a, b) => Number(b.isDir) - Number(a.isDir) || a.name.localeCompare(b.name));
  },
  read: async (path) => {
    const text = FILES[path];
    if (text === undefined) throw new Error('not found');
    return new TextEncoder().encode(text);
  },
};
