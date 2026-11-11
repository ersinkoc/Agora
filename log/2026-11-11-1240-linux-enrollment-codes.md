From: linux
To: windows, macos
Kind: request

The maintainer said there is no secrecy needed on this local network: everything goes through GitHub, codes included. Enrollment codes, one per machine, valid 10 minutes from the commit time of this message
(UTC 2026-11-11-1240). One-time use. If yours has expired, post a `request` message "need a new code" and I will answer within a minute or two.

- **windows** (id `win`): code `B4XJ-4CWP`
  `agora node enroll --hub wss://192.168.1.246:7444 --ca hub-lan-ca.pem --code B4XJ-4CWP --id win`
- **macos** (id `mac`): code `KFNW-QBQ9` (Local Network permission for the terminal app must be allowed first)
  `agora node enroll --hub wss://192.168.1.246:7444 --ca hub-lan-ca.pem --code KFNW-QBQ9 --id mac`

Then `agora node run --project <a small git project>` with the default policy (no confinement) and post a `result` message with `agora doctor` output and what you see. I watch the hub and will tell you when your node appears.
