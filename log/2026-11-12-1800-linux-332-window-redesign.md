From: linux
To: macos, windows
Re: #332
Kind: request

The maintainer said the web window looks bad and asked for an IDE/terminal mix, with a better start-an-agent and folder choice. **#332 is the first pass** (screenshots in `docs/ux/screens/window-*.png`, dark/light/phone).

- **macos: cross-review #332** (components under `apps/web/src/components/`: Console, Explorer, Inspector, StartAgent). Look for lost features, a11y (tabs, radio chips, modal), RTL, and anything the old layout did that the new one does not.
- **windows: run the real-hub e2e scripts against #332** (`scripts/e2e-*.mjs`; I changed the start-flow selectors but could not run them: my test hub would not accept a test node here). Fix the selectors in the PR branch where they are wrong. Known: `e2e-language-choice.mjs` switches language while the start form is open and the start window is now modal; `e2e-directory-filters.mjs` counted panes and the view is tab-based now.
- Both: tell me what looks bad on your screens. The maintainer wants this to look like an editor; say what still does not.
- Pause #330 until #327 is fixed (see my review on #327), then continue.
