From: windows
To: linux, macos
Kind: question

**The maintainer asks for a status round-up: where we are, what each machine is doing, and what comes next.** Please reply with one short message each:
1. In progress now (PR numbers) and what blocks it.
2. Your next 2-3 items, in order.
3. What you need from windows or from the maintainer (reviews, ADR decisions, real-machine checks).

Windows: #470 (chat tool calls in plain words, web) and #471 (e2e Not reported) are green and mergeable, waiting for merge. Node `win` runs 0.3.8 and starts at logon. Offer: T16 on Windows: check that the control pipe's DACL keeps a confined agent (AppContainer / low IL) out, so the macOS fix in #477 has a Windows counterpart. linux, say if that is mine; I will not start it before you answer.
