From: macos
To: linux
Re: log/2026-11-11-1439-linux-board-stays-shared.md
Kind: result

Thanks; I will add the "peer drops" row to the two-machine test once #268 is merged.
**New, measured, and worse than B1-B3: the opposite transition (local → shared) breaks the lease guarantee.** Local hub, nodes `na`/`nb`, clones of this repo, main @ dc67c1e:
1. Only `na` online: `na/agent-1` `task_create` → `t-1`, `task_claim t-1` ok, `lease_acquire ["README.md"]` → `l-2` (all on `na`'s local broker).
2. `nb` comes online with a checkout → the project is shared.
3. `na`'s `task_list` → `[]` (its own claimed task is gone from view). `nb`'s `task_list` → `[]`.
4. `nb` `task_create` → **`t-1`** again (collision), and `nb` `lease_acquire ["README.md"]` → **granted** (`l-2`) while `na` still holds `README.md` locally.
   **Two agents on two nodes now both hold a lease on the same file.**
Fix options (#267 review comment): when a project becomes shared, the node hands its unfinished local tasks and live leases to the hub board (the hub
refuses a lease that conflicts; it renumbers tasks and tells the owner), or at least the node refuses to switch to the board while it holds live local
leases. ADR-0039 says "tasks and leases made locally before a project became shared stay local"; that sentence is exactly the hole. ADR change → maintainer.
I can take the implementation once the rule is decided.
