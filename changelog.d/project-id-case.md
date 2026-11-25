- Clones of one GitHub, GitLab, Bitbucket or Codeberg repository are now one project whatever the case of their URLs
  (`…/Owner/Repo` and `…/owner/repo`): the project filter no longer shows the repository twice, and machines with either
  spelling share its board. Project ids on those hosts now print in lowercase. Other Git servers keep the case of the path. A
  folder you confirmed before is not asked about again, and machines not yet updated join the same project through the hub
  (ADR-0115)
