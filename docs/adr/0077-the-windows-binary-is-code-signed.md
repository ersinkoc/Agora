# 0077. The Windows binary is code-signed
Status: Proposed
Date: 2026-11-15
Origin: the move of `win` to `agora service` on 0.3.12 (LAN finding L13, #588). Microsoft Defender flagged the verified release
`agora.exe` (`Behavior:Win32/Execution.A!ml`) within a few minutes of `agora service install`, twice. It quarantined the file, ended
the node and removed the logon task. Binaries built locally from the same code were never flagged.

## Context
- Releases are signed with the project's ssh key (ADR-0019): `agora update` and `agora hub ssh install` verify it. Windows knows
  nothing of that key. To Windows, every Agora release is an unsigned program from a publisher it has never seen.
- `agora service install` does exactly what persistence malware does: an unsigned, freshly downloaded program replaces itself
  (`agora update`), registers a logon task and runs itself under a hidden console (`conhost --headless`). Defender's
  machine-learning verdict is about that behaviour from a file with no reputation. Changing our code would not give the file a
  reputation, and the console wrapper exists because a logon task that starts a console program opens a visible window
  (measured in M7; ADR-0024 rules that window out).
- ADR-0048 already says the desktop app "needs a signing story per platform … that costs money and accounts only the maintainer has;
  unsigned builds are published as such until then". This ADR is that story for Windows, for `agora.exe` and the desktop app alike.
- **Reputation facts that shape the choice** (Microsoft's documentation, 2026): an EV certificate no longer gives instant SmartScreen
  reputation (removed in 2024); OV and EV are treated alike for new files. Reputation is earned per file hash, and over time per
  signing identity. So any certificate helps only after a few releases signed by the same identity. A signature also lets a
  false-positive report name a publisher.
- **Smart App Control** (Windows 11, on or in evaluation mode, mostly on clean installs) blocks an unsigned program without
  reputation outright. That is not a Defender detection: there is no "Allow on device" and no per-file exception, only switching
  Smart App Control off for the whole machine, which on most builds cannot be switched back on without resetting Windows. A
  signature from a certificate authority Windows trusts is what it accepts. (Microsoft's documentation; not measured here, since
  this machine has it off: `VerifiedAndReputablePolicyState = 0`.)

## Options
1. **SignPath Foundation (free for open-source projects).** The repository is Apache-2.0, which is OSI-approved. A GitHub Action
   submits the release artifact; SignPath checks that it came from the named workflow run, and signs it in their HSM.
   - Costs: the certificate names **SignPath Foundation** as the publisher, not the maintainer. The project must say "Free code
     signing provided by SignPath.io, certificate by SignPath Foundation". Every team member uses MFA on GitHub and SignPath.
     Approval is a manual review by SignPath and not guaranteed.
   - No money, no key on our side.
2. **Microsoft Azure Artifact Signing (formerly Trusted Signing).** A Microsoft-run certificate in the maintainer's (or a company's)
   name, signing from GitHub Actions with a monthly subscription (two tiers by signature count; billed from the day the account is
   created). Whether an individual, as opposed to an organisation, can complete its identity validation must be checked before
   choosing it.
3. **A purchased OV certificate.** In the maintainer's name, yearly cost. Since 2023 the private key must live in a hardware token
   or a cloud HSM, so CI signing needs a cloud-HSM service (an extra account and cost). An EV certificate costs more and, since
   2024, no longer brings instant reputation.
4. **Stay unsigned** (today). Document it (#588). Users choose "Allow on device" in Protection history. On a machine with Smart App
   Control on, this is a dead end: `agora.exe` does not run at all, and the only way out is turning Smart App Control off for good. Optionally submit each
   release to Microsoft as a false positive (from the maintainer's account; each release is a new hash).

## Decision (proposed)
Option 1, SignPath Foundation, as the first step: it costs nothing, keeps the key out of our hands and can run inside the existing
release workflow. If SignPath does not accept the project, or the maintainer wants their own name as the publisher, option 2.
Until a signed release exists, option 4 stays, as documented.

**What changes when it is accepted:**
- `.github/workflows/release.yml`: the Windows `agora.exe` (and later the desktop installer) is uploaded as an artifact, signed
  through SignPath's action, timestamped, and the **signed** file is what gets the `.sha256` and the ssh signature. ADR-0019's
  verification is unchanged; Authenticode sits on top of it.
- The release notes and README name the publisher shown in Windows' file properties, so users can check it.
- `agora update` keeps verifying the ssh signature. It does not check Authenticode (Windows does that on its own).

## Consequences
- Windows sees a known publisher. Defender and SmartScreen build reputation for that identity release by release. The first signed
  releases may still be flagged; the documentation (#588) stays until they are not.
- With option 1 the publisher name is not the project's own. With option 2 it is the maintainer's, for a monthly fee.
- Release time grows by one signing round trip.
- macOS notarization is a separate decision (ADR-0048) and is not covered here.

## Open questions (for the maintainer)
1. Option 1 (free, publisher "SignPath Foundation") or option 2 (paid, your own name)?
2. Is it acceptable that every team member, the agents' GitHub use included, needs MFA on the accounts SignPath checks?
