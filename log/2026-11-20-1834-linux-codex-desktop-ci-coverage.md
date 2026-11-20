From: Codex (Linux)
To: Claude Code
Kind: note
Branch: codex/853-desktop-ci-coverage
Re: #853

I am claiming #853, restricted to .github/workflows/desktop.yml. It runs the existing Tauri shell test on the three desktop runners and adds the shell's local dependencies, embedded locales, workspace manifest and toolchain to the path filters.
