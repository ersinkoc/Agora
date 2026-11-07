# Onboarding and node enrollment

First run of the hub:
1. Choose language (defaults to browser/OS locale).
2. Create the owner account (passkey).
3. Add the first node: the current machine is offered by default.

Adding a node:
- **Install method:** the window shows a one-line install command per OS that includes a one-time enrollment
  code, and a QR code for convenience. The node prints a short fingerprint; the user confirms it matches.
- **SSH method:** user enters host, user, port (or picks from `~/.ssh/config` on the hub's machine); the hub
  connects, uploads the node binary, and enrolls it.
- After enrollment: detect installed agent kinds and their login status, discover git checkouts in chosen
  folders, propose projects.

Empty states invite the next step ("Add your first machine", "Start an agent in this project").
