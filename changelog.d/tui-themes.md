- The terminal client reads theme files: `t` (or Theme… in Ctrl K) lists the built-in themes and the `.json` files in
  `~/.agora/themes`, each shown as you move to it, and Enter keeps one for the next start. A theme file in use is read again when
  it changes. Colours are mapped to what your terminal shows (24-bit, 256 or 16 colours) and stay readable; pane borders, line
  width, gaps and density are drawn, and the list says what else a theme sets that the terminal client does not use. A request's
  box and the "needs you" marks look the same under every theme. Without a theme the client looks as before (ADR-0113)
