- The terminal client reads a theme file that starts with a byte-order mark (as Windows PowerShell 5.1 and older Notepad save UTF-8),
  or that has a byte that is not UTF-8, as the web window does, instead of refusing it as "not JSON". (audit F1800)
