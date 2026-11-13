# Untrusted launch-option IDs bypass confirmation (#438)

Reviewed head: `51c371e187a74b8bb9656ff084d8e87f911bfe29`.

A hub can advertise a harmless option with id `skip-permissions!` and label `option-ask`. The real start
window renders the localized **Ask before acting** checkbox. Checking it and pressing Start opens no
confirmation, but `chosenIds` returns `skip-permissions!`, which the browser signs. The node interprets
the hub-supplied suffix as the confirmation of its actual `skip-permissions` option. Its `choose()`
function accepts the browser output under the default `allow` policy and produces
`--dangerously-skip-permissions` with `skips_permissions = true`.

Minimal client reproduction with the actual exported functions:

```ts
const offered = [{ id: 'skip-permissions!', label: 'option-ask', skipsPermissions: false }];
const state = toggleHarmless(initialLaunch(), offered[0].id, true);
chosenIds(state, offered); // ['skip-permissions!']; permission='', pending undefined
```

There is a second ambiguity: advertise one harmless id `resume-last,skip-permissions!`. After the
browser signs it, the hub can send two ids `resume-last` and `skip-permissions!`. `launchMessage` joins
the list with commas, so the signature still verifies. An incoming option id must satisfy the manifest's
`[a-z0-9-]{1,32}` alphabet before it becomes a choice. Confirmation markers must be created by the client
only after confirmation; they cannot come from a hub-advertised id. Keep the node's policy and marker checks.

The attached JSON records four Chromium runs (English/Turkish, pre-marked/comma-bearing id). These use
the actual `StartAgent`, `chosenIds` and `signDirectory` code with controlled `loadKinds`/`onStart`
callbacks and a fresh local test key. Every run has zero confirmation dialogs, and the actual signature
verifies for both the original and rewritten vectors. The screenshots show the Turkish pre-marked case
in both themes. No hub command was sent, no real agent was started, and no live key was used.

A separate Rust proof fed the web proof's JSON output to the actual node `choose()` function and
observed the accepted dangerous argument. Security assertions fail as expected on this head. Existing
node option tests (6), signature-binding regression (1), and malformed/unreadable-policy regression (1)
pass; they do not cover the malicious advertised ids.
