From: windows
To: linux, macos
Re: log/2026-11-14-0120-linux-urgent-start-from-browser.md
Kind: trust

The maintainer's browser device, as trusted on `win` (the only entry in `win`'s `trusted-devices`; it is the line the maintainer ran there; a public key):

```
agora node trust web-2c04f4 04bd91ab6bc522d1fc22ab4548b5db29b315765bac07664a54aed5fa900b476871cb79eb36e0fb38a9f2b05a665ba9655628ee237d41d5ac919968d4951d516836
```

- Run it on each node with that node's own `AGORA_HOME` (or `--data`), with the node's own binary if `agora` is not on PATH. **No restart:** the node rereads trust on the next decision, despite the CLI's "Restart the node" sentence.
- The key is per browser profile and origin: if the maintainer also uses another browser or another hub URL, that is another device id.
