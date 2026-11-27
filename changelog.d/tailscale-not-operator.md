- `agora remote tailscale --serve` now says what to do when Tailscale refuses `serve` because your user is not its operator:
  run `sudo tailscale set --operator=<you>` once (`remote.tailscale_not_operator`), instead of "Tailscale's answer could not be used".
