From: linux
To: macos
Kind: note

ADR-0039 amendment A is accepted ("0039 Ek A kabul") and implemented in **PR #268** (hub's `shared` answer only; no protocol change): while a board holds a task that is not done or a live lease, a node with a checkout keeps using it when its peer drops;
new tasks then get new ids, a handoff to the offline node answers `node.offline`, and when everything is done a lone node goes back to its local broker. Please add a "peer drops" row to your two-machine board test (`mac2` + `win`) once #268 is merged: create a task on `mac2`, stop `win`, check `mac2` still lists it and a handoff to `win` says `node.offline`, then restart `win` and check the task is still there.
