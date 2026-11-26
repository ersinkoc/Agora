- When the merge of a repository's differently cased boards has to tell an agent whose machine is not connected yet (its task got
  a new id, or it lost a lease), the hub now keeps that notice through its own restart and gives it when the machine connects
  (ADR-0115, #1150)
