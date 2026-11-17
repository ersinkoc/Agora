# Linux audit to Linux merger: round 6, ready to merge #729, #730, #732, #733

Every CI job is green on the current heads. Codex (read-only) reviewed each PR over several rounds, and every finding is fixed or answered in the PR body.

- **Ready to merge: #729** (head 0bb7dd5a). F980 Med: a folder whose path is not UTF-8 is refused (before, it was named as another folder). F981 Med: `.agent-project.json` is read through a handle that never follows a link (new `agora_platform::open_read_no_follow`, with Unix and Windows forms); before, a link to /dev/zero, a FIFO or a file elsewhere was read.
- **Ready to merge: #730** (head 7d57551e). F990 Med: a forgotten agent stays hidden in the live view, and the client mirrors the hub's per-agent forgotten flag. F992 Low: a restarted machine keeps its memory and terminal counts. F991 was withdrawn.
- **Ready to merge: #732** (head 34f8f214). F961 Med: `agora --listen` on a running machine no longer fails every later run, because the front door follows the running hub's recorded port. F962 Med: `agora down` refuses a linked `run` folder before stopping anything. F960 was withdrawn: a hub bound only to a non-loopback address needs the maintainer's decision.
- **Ready to merge: #733** (head 6f051bbc). F970 Med: a node can dial a hub at an IPv6 address. F971 Low: `agora node trust --from-hub` and `agora node enroll` give up on a silent hub after 25 s.

#725 and #726 from round 5 stay ready. Round 7 (hubcli enrollment, limited-device folders) is running. I do not merge.
