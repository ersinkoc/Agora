# Strict TLS certificate validation (#319)

During local deployment, Python 3.14's default SSL context rejected the hub server
certificate with `Missing Authority Key Identifier`. A public-only probe confirmed
that the existing CA certificate itself passes `openssl verify -x509_strict`, while
the live server certificate fails with error 85. No installed private key was read.

Both `Ca::issue_server` and `Ca::issue_node` used rcgen's default omission of that
extension. Each now enables `use_authority_key_identifier_extension`. rcgen derives
the value from the issuer, including a pre-existing custom Subject Key Identifier
when loading the same CA from PEM. No CA certificate/key or trust identity is changed.

This implements the certificate profile in
[RFC 5280 §4.2.1.1](https://www.rfc-editor.org/rfc/rfc5280.html#section-4.2.1.1).
[OpenSSL's strict verifier](https://docs.openssl.org/3.5/man1/openssl-verification-options/)
checks for that identifier in non-self-signed X.509v3 certificates. This does not add
protocol fields, dependencies, enrollment authority or a different revocation policy.

## Verification

Four tests in `crates/agora-hub/tests/ca_key_identifiers.rs` cover:

- Server and node leaves each have exactly one non-critical issuer identifier equal
  to the CA's Subject Key Identifier, with a valid signature.
- Both newly generated and PEM-reloaded authorities issue those leaves without
  modifying their CA certificate bytes.
- An authority loaded with a custom Subject Key Identifier retains that identifier
  in both leaf types, rather than substituting a new default hash.
- Independent `openssl verify -x509_strict` accepts server/client purposes and the
  correct server hostname; wrong hostnames and a client certificate used as a server
  remain refused. This external check runs when an OpenSSL executable is available;
  the portable structural/signature checks always run. It writes public, disposable
  certificates only, never private keys or real credentials.

The initial three tests fail before the fix; all four pass afterwards. Normal
workspace TLS/enrollment tests also verify compatibility. Full workspace tests,
Clippy with warnings denied, formatting, locale and docs checks pass locally;
final CI results are recorded in the PR.

The server's next normal startup issues its new leaf using the existing CA. New or
renewed nodes receive the identifier without forced reenrollment. This PR does not
regenerate trust material, revoke existing certificates, restart the live node or
install unreviewed auth code. Independent cross-review and maintainer merge remain.
