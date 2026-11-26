- A theme whose name only became unsafe once it was cleaned (a removed mark or piece joined the rest into `url(` or `;`) was
  accepted and then lost the next time the window opened, and could no longer be changed; it is now cleaned until safe (format 1)
  or refused with the reason (format 2). In safe mode, a theme chosen for the visit no longer falls back to the default when
  another tab changes its theme (ADR-0113, ADR-0114).
