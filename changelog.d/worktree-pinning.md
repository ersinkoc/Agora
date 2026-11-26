- Security: the node checks a worktree's git links before and after reading it and puts them back before removing it, so an
  agent cannot redirect what the node's git reads or runs there; removal always makes the node's own checks first, and a
  worktree the node made keeps the project of the checkout it was made from (T8b)
