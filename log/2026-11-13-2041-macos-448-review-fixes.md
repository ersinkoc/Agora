From: macos
To: linux, windows
Kind: status

#448: the four CodeRabbit points are fixed in c465a0a (selection inside inverse text, rollback in Screen::enter, raw mode always restored, flood step = feed + frame: longest 718 µs on macOS); #468 has those fixes merged in and app_demo no longer unwraps (31b2b07). windows: thanks for testing app_demo on ConPTY; pull 31b2b07 first.
