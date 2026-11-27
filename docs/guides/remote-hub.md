# Reach your hub from a phone or another city

ADR-0067 rungs 3 and 4. This guide puts Agora on a server (a VPS with no screen) and opens it from your phone or laptop. Three ways are
described (the third, a tunnel, is section C). **Use Tailscale if you can**: only the devices of your tailnet (yours, and anyone you shared the machine or tailnet with) can reach
the hub. A plain public address works too, but then anyone on the internet can reach it and only the screen password and the
certificate protect it.

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
If Tailscale Funnel is on (it puts something on the whole internet), `agora remote status` says `Public internet: reachable through Tailscale Funnel` and `agora doctor` warns; turn it off with `tailscale funnel reset`.
`agora doctor` checks the same things: Tailscale answers, a password is set while a way in is on, and the hub's certificate names
the tailnet name (a new name reaches the running hub's certificate within a few seconds).

Without `--serve`, `agora remote tailscale` only adds the names (for other machines of yours that dial the hub over the tailnet,
for example `agora join myserver.tail1234.ts.net:7443`); the hub must then listen on the machine's tailnet address (`agora --listen 100.x.y.z:7443`, the address `tailscale ip -4` prints; not `0.0.0.0`, which also opens the server's public address), or stay on loopback and use `--serve`.

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

A hub that is not started on a public address with no password stays that way only as long as it runs: the check is made when the hub starts, so look at `agora doctor` after a network change. A hub on a public address says so, again and again: `agora remote status` and `agora doctor` both print `The hub listens on a
public address. Only the screen password and the certificate ... protect it ... Tailscale is the safer way`, as long as it
does. A hub cannot be started on a public address with no password, and `agora password clear` refuses while a way in from
outside is on.

## C. Server and a tunnel or reverse proxy (Cloudflare Tunnel, or any proxy you run)

ADR-0067 rung 4. Use this when the phone or laptop cannot be on your tailnet and you do not want to open the hub's port to the
internet: a connector on the server (for a Cloudflare Tunnel, `cloudflared`) dials out to the provider, and the provider serves
your own host name with a real certificate. **The hub is then reachable from the whole internet at that name.** Read the next
paragraph before you turn it on.

**Put an access layer with single sign-on in front of the host name first. This is strongly recommended.** With Cloudflare, that
is Cloudflare Access: in the Zero Trust dashboard add a "self-hosted" application for the host name and a policy that allows only
your own account. Then only people you named even reach the Agora sign-in page, and the screen password and the sign-in code are
a second lock instead of the only one. Without it, anyone on the internet can try codes and passwords at the hub, held back only
by the lockouts of rung 1. Agora cannot check that you did this; it only says so, every time (`agora remote status`, `agora doctor`).

What the hub does when you turn it on (`agora remote proxy`), and only then:

- It trusts **one** header for a visitor's real address, the one you name, and only on a connection from this machine (the
  connector) or from a proxy address you list. Anyone else's copy of that header is ignored. The proxy must set the header itself
  on every request and overwrite what a visitor sent; never name `X-Forwarded-For`, which proxies append to.
- A proxied visitor is never "the owner at the machine": the screen password and the sign-in code apply to everyone who comes
  through it, and each visitor address has its own lockout (without this, they would all share one).
- It answers only the host names you give (and the machine's own loopback names). Another `Host`, or a browser `Origin` of another
  site, is refused (`421` / `403`), the live connection (WebSocket) included.

Steps, on the server (the hub must already run on loopback with a screen password; see A above):

1. **Set the screen password** if you have not: `agora password set`.
2. **Create the tunnel with the provider's own tools** (Agora holds no Cloudflare credentials and does not run `cloudflared`).
   For `cloudflared`, a configuration along these lines points the host name at the hub on loopback. The hub speaks HTTPS with
   its own certificate, which `cloudflared` cannot check, so it is told not to:
   ```
   tunnel: YOUR-TUNNEL-ID
   credentials-file: /home/you/.cloudflared/YOUR-TUNNEL-ID.json
   ingress:
     - hostname: agora.example.com
       service: https://127.0.0.1:7443
       originRequest:
         noTLSVerify: true
     - service: http_status:404
   ```
   Use the port `agora remote status` prints for "The hub listens on". Start the Cloudflare Access application for the same host
   name now (above).
3. **Tell the hub to trust it:**
   ```
   agora remote proxy --trust-header CF-Connecting-IP --host agora.example.com
   ```
   The header name `CF-Connecting-IP` is the one Cloudflare documents for the visitor's address. Use your provider's, or your
   proxy's own, if you do not use Cloudflare, and `--proxy-addr IP` (up to 4) when the proxy connects from another machine
   instead of from this one. It prints the names and the header and repeats the Access recommendation. The running hub takes it
   within a few seconds; no restart.
4. **Open `https://agora.example.com/`**, pass the access layer, then sign in with `agora hub login`'s code and the screen password.

`agora remote status` shows `Trusted proxy: on, answers agora.example.com, real address from the "cf-connecting-ip" header` and
the internet warning; `agora doctor` warns every time, fails if the password is missing, and warns if the hub also listens beyond
this machine (start it on `127.0.0.1` so that the proxy is the only way in). **Turn it off:** `agora remote off` removes the trust
(the connector keeps running until you stop it; stop it and delete the Access application too).

**Check it from outside, and know the one gap.** The hub cannot tell the owner's own browser at the machine from a proxy that forwards to `127.0.0.1` with that `Host`, sends no address header and adds no other proxy header (nginx's `proxy_pass` does exactly this unless you set `Host` and `proxy_set_header`): such a request is taken for the owner and is not asked for the screen password. (Only a POST or a WebSocket from the page of the public name is caught, because its `Origin` names the public host; a plain page load carries no `Origin`, so the gap remains there.) A tunnel connector sends the public host name, which is refused the owner's rights, but with any other proxy set the public `Host` and the address header, and after turning it on open the host name from a phone on mobile data to see the sign-in ask for the password.

**With Tailscale serve at the same time.** `tailscale serve` visitors also arrive from loopback, with the machine's `.ts.net` name as `Host`. While the proxy is on, that name must be one of your `--host` names or they get `421`; and since `tailscale serve` sets no address header they all share one unknown-address sign-in bucket, as before.

**A damaged settings file.** If `<hub dir>/remote-proxy` is damaged, a running hub keeps the last good settings, including the Host and Origin checks, until the file is good or removed; a hub that starts with it damaged trusts no proxy and checks no Host or Origin. `agora remote status` and `agora doctor` say so; fix it with `agora remote off` and `agora remote proxy` again.

**What this does not do.** The nodes of your other machines connect to the hub directly with their own certificates; they do not
go through this tunnel, and this guide does not set that up. The window does not yet show a banner while the proxy is on (the
warnings are in the terminal commands). `agora remote proxy` does not start or stop the connector.

**What was not tested against a real tunnel.** All of the above was tested with a fake proxy: a real `agora` hub process and
real sockets on this machine, with the headers a proxy would send written by hand. Nobody has yet run it behind a real
Cloudflare Tunnel, or behind another real proxy. In particular these are taken from the provider's documentation and not
confirmed: that `CF-Connecting-IP` is set on every request and overwrites a visitor's own copy, that the `Host` that reaches the hub
is the public name (the connector's default), that `noTLSVerify` is accepted for the hub's certificate, and that WebSocket
upgrades and `Origin` pass through as a browser sends them. If your `Host` or `Origin` arrives as something else, the hub answers
`421`/`403` and nothing works, which is the safe failure; capture what arrives before you loosen anything. A phone on a real
mobile network was not tried either.

Not covered here: installing coding agents from the window (ADR-0087).

## If something goes wrong

| You see | Do |
| --- | --- |
| `remote.password_required` | `agora password set`, then repeat the command |
| `remote.tailscale_missing` / `remote.tailscale_logged_out` | install Tailscale / `tailscale up` |
| `remote.tailscale_unreadable (bad_name)` | the machine name Tailscale reports is not a plain `.ts.net` name; Agora does not use it. Rename the machine in Tailscale |
| `remote.tailscale_not_operator` | on Linux, Tailscale lets only root or its operator use `tailscale serve`: run `sudo tailscale set --operator=$USER` once, then the command again |
| `remote.serve_port_in_use` | choose another port: `agora remote tailscale --serve --port 9443` |
| `remote.password_in_use` | `agora remote off` first, then `agora password clear` |
| `remote.proxy_header_invalid` / `remote.proxy_host_invalid` / `remote.proxy_addr_invalid` | give a plain header name, plain public host names (no address, wildcard or port) and IP addresses |
| The browser shows `421` / `403` (`proxy.host_not_allowed`, `proxy.origin_not_allowed`) | the name in the address bar is not one you gave `--host`, or the proxy changes `Host`/`Origin`; run `agora remote proxy` again with the right name |
| `agora remote status` says the proxy file is damaged | `agora remote off`, then `agora remote proxy` again |
| A way in is on but you forgot the password | on the server: `agora remote off`, then `agora password set` |
