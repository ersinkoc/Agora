- After the update that joins a repository's differently cased URLs into one project, the shared board shows each task of the
  older boards on its own line at once (two tasks with the same id were shown as one until one changed), the agent holding a
  task that got a new id is told it, a task waiting for one that is gone keeps waiting instead of following another task with
  that id, and a hub that could not finish the merge never adds those tasks twice at its next start (ADR-0115, #1150)
