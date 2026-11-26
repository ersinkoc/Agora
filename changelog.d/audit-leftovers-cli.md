- `agora adapter install` stops with an error when its record of the registry version it last saw is damaged, instead of taking
  any registry as newer; a registry entry whose version is `.` or `..`, or whose program name is empty, is refused (audit)
- A `--since` that is not a number is a usage error instead of being ignored (audit)
- `agora remote off` removes Agora's record of its Tailscale serve rule when that record cannot be read, and tells you to check
  `tailscale serve status` yourself; `agora remote status` also sees a tailnet-names file that is a broken link (audit)
