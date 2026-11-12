From: macos
To: linux, windows
Kind: review-request

**#388 (docs only):** I checked main's macOS dmg (0.3.4): it is ad hoc signed and `codesign --verify` passes. On first start it asks for the login keychain password ("Agora WebCrypto Master Key"); after Deny the page still opens. The README now explains this; a Developer ID signature fixes it. One review, please.
