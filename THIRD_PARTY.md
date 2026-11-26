# Third-party data and code

| Component | Use | License | Source |
| --- | --- | --- | --- |
| models.dev | Model metadata and per-token prices (ADR-0021); a trimmed snapshot is embedded in the hub (`crates/agora-hub/data/prices.json`) | MIT | https://github.com/sst/models.dev |
| Catppuccin palette | Colour values of Catppuccin Mocha in the starter theme `apps/web/src/themes/starter/catppuccin-mocha.json` | MIT, Copyright (c) 2021 Catppuccin | https://github.com/catppuccin/palette |
| Dracula | Colour values of Dracula (its palette, and its terminal colours from the official Alacritty port) in `apps/web/src/themes/starter/dracula.json` | MIT, Copyright (c) 2023 Dracula Theme | https://github.com/dracula/dracula-theme, https://github.com/dracula/alacritty |
| Gruvbox | Colour values of Gruvbox dark and light in `apps/web/src/themes/starter/gruvbox.json` | MIT (the original's README and package.json say MIT/X11; the community repository's LICENSE: Copyright (c) 2018 Pavel Pertsev) | https://github.com/morhetz/gruvbox, https://github.com/gruvbox-community/gruvbox |
| Rosé Pine palette | Colour values of Rosé Pine (main) in `apps/web/src/themes/starter/rose-pine.json` | MIT, Copyright (c) mvllow | https://github.com/rose-pine/rose-pine-palette |
| Tokyo Night (Neovim) | Colour values of Tokyo Night (the Night style, from its generated `extras/lua/tokyonight_night.lua`) in `apps/web/src/themes/starter/tokyo-night.json` | Apache-2.0, by Folke Lemaitre (the LICENSE names no copyright line; the generated extras say MIT) | https://github.com/folke/tokyonight.nvim |

Rust dependencies are checked by `cargo deny` (`deny.toml` lists the licenses allowed in the dependency tree; the desktop shell has its own list in `apps/desktop/src-tauri/deny.toml`). JavaScript dependencies are pinned by `apps/web/package-lock.json` and are not license-checked by CI yet; ADR-0036 asks for the web client's third-party packages to be listed here, which is still to do. The project's own license is Apache-2.0 (`LICENSE`, ADR-0016); Apache-2.0 needs no `NOTICE` file, and this repository has none.

**Starter themes** (`apps/web/src/themes/starter/`, `docs/ux/themes.md`) hold only colour values (hex) published by each palette's
project, placed in Agora's own theme format; no theme file of those projects is copied. The licences above allow this with the notice
kept: each project's copyright line is in the table, and the MIT licence text is:

> Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files
> (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge,
> publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do
> so, subject to the following conditions: The above copyright notice and this permission notice shall be included in all copies or
> substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
> BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
> OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

Tokyo Night's Apache-2.0 is the project's own licence (`LICENSE`); the repository has no `NOTICE` file to carry.
