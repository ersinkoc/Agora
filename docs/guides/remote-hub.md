# Reach your hub from a phone or another city

ADR-0067 rung 3. This guide puts Agora on a server (a VPS with no screen) and opens it from your phone or laptop. Two ways are
described. **Use Tailscale if you can**: only your own devices can even find the hub. A plain public address works too, but then
only the screen password and the certificate protect it.

Either way the rule is the same: **nothing is reachable from outside until you set a screen password**, and every sign-in from
another machine needs it (plus a sign-in code). `agora remote tailscale`, a hub on a public address and the first run on a
server all refuse without one, in words, and say what to type: `agora password set`.

## What you need

- A server you can reach by SSH (Linux, with `systemd`; the commands below are for it), and the `agora` binary on its `PATH`.
- For the Tailscale way: Tailscale installed and signed in on the server and on your phone or laptop (`tailscale up`), with
  MagicDNS and HTTPS certificates turned on for your tailnet (the Tailscale admin page, DNS tab).

## A. Server and Tailscale (recommended)

The hub stays on the server's own loopback address. Tailscale gives it a real HTTPS name, so there is no certificate warning.

1. **Start Agora on the server**, from an SSH session:
   ```
   agora --ui none
   ```
   The first run asks one question; answer `1` ("Start Agora here"), then `n` to "Let other machines of yours connect?" (the hub
   stays on this machine). `--ui none` starts the hub and the node and does not try to open a window, which a server has none of.
2. **Set the screen password** (it asks twice and shows nothing as you type; at least 8 characters):
   ```
   agora password set
   ```
   It prints `Screen password set. A sign-in from another machine now needs it; ...`.
3. **Turn on the Tailscale way in:**
   ```
   agora remote tailscale --serve
   ```
   It prints the names it added to the hub's certificate and the address to open:
   ```
   The hub's certificate now names this machine on your tailnet: myserver.tail1234.ts.net, 100.101.102.103, ...
   Open Agora from a phone or laptop on your tailnet at https://myserver.tail1234.ts.net:8443/ (sign in with a code and the screen password).
   ```
   `--serve` publishes the hub with `tailscale serve` on HTTPS port 8443 (`--port N` for another). Agora adds that one rule and
   remembers it; it never changes your other `tailscale serve` rules, and it stops with `remote.serve_port_in_use` if the port is
   already yours. If Tailscale is not installed or not signed in, it says so (`remote.tailscale_missing`,
   `remote.tailscale_logged_out`) and changes nothing. If it waits for you to allow HTTPS on your tailnet, open the link
   `tailscale serve status` shows once and run it again.
4. **Get a sign-in code** on the server: `agora hub login` prints `Sign-in code for the web app: ... (valid for 10 minutes).`
5. **Open the address on your phone** (the phone needs Tailscale on). Enter the code and the screen password. Add the page to your
   home screen if you like.

Look at it any time with `agora remote status`:
```
The hub listens on 127.0.0.1:7443.
This machine: reachable.
Local network: not reachable.
Tailscale: reachable.
Names in the hub's certificate for it: myserver.tail1234.ts.net, 100.101.102.103
Tailscale serve: https://myserver.tail1234.ts.net:8443/
Public internet: not reachable.
Screen password: set.
```
`agora doctor` checks the same things: Tailscale answers, a password is set while a way in is on, and the hub's certificate names
the tailnet name (a new name reaches the running hub's certificate within a few seconds).

Without `--serve`, `agora remote tailscale` only adds the names (for other machines of yours that dial the hub over the tailnet,
for example `agora join myserver.tail1234.ts.net:7443`); the hub must then listen on the tailnet address (`--listen 0.0.0.0:7443`).

**Turn it off:** `agora remote off` removes the names from the list (the certificate drops them the next time the hub starts) and
removes the one serve rule Agora added. A rule you changed or added yourself, on that port or any other, is left exactly as it is.

## B. Server and a plain public address

Use this only when Tailscale is not an option. Anyone on the internet can reach the hub's port and try a code and a password;
what stops them is the screen password (slow hash, growing lockout, audited) and the sign-in code. The certificate is the hub's
own, so a browser warns until you trust it; check the fingerprint (`agora hub fingerprint`) against what the browser shows.

1. **First run, naming the public address and port**, then stop at the password:
   ```
   agora --ui none --listen 0.0.0.0:7444
   ```
   (`[::]:7444` for IPv6 too.) Answer `1`, then `y`. It saves the address and refuses to start the hub, because it would be open to
   the internet:
   ```
   Error remote.password_required: No way in from outside is turned on without a screen password. Set one first with `agora password set`, ...
   ```
2. **Set the password**, then start for real:
   ```
   agora password set
   agora --ui none
   ```
3. **Keep it running across logouts and reboots:**
   ```
   agora service install
   sudo loginctl enable-linger "$USER"
   ```
   `agora service install` registers a user service for the hub and the node (no administrator rights). On Linux a user service
   only runs while you are logged in unless lingering is on; Agora never turns it on by itself and prints the line to run. If it
   says there is no user service manager in this session, log in again with a normal SSH session (not `su`), or run
   `systemctl --user` once to check it works.
4. **Open `https://SERVER-ADDRESS:7444/`**, accept the certificate warning after checking the fingerprint, and sign in with
   `agora hub login`'s code and the screen password.

A hub on a public address says so, again and again: `agora remote status` and `agora doctor` both print `The hub listens on a
public address. Only the screen password and the certificate ... protect it ... Tailscale is the safer way`, as long as it
does. A hub cannot be started on a public address with no password, and `agora password clear` refuses while a way in from
outside is on.

Not covered here: a Cloudflare Tunnel (a later step of ADR-0067), and installing coding agents from the window (ADR-0087).

## If something goes wrong

| You see | Do |
| --- | --- |
| `remote.password_required` | `agora password set`, then repeat the command |
| `remote.tailscale_missing` / `remote.tailscale_logged_out` | install Tailscale / `tailscale up` |
| `remote.tailscale_unreadable (bad_name)` | the machine name Tailscale reports is not a plain `.ts.net` name; Agora does not use it. Rename the machine in Tailscale |
| `remote.serve_port_in_use` | choose another port: `agora remote tailscale --serve --port 9443` |
| `remote.password_in_use` | `agora remote off` first, then `agora password clear` |
| A way in is on but you forgot the password | on the server: `agora remote off`, then `agora password set` |
