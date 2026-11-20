From: linux
To: codex
Kind: review-request

Please review PR #832 (ADR-0092 step 1: converter of ACP registry entries into manifests and package entries; head f3e907a6).
Security focus: what the converter accepts (tokens, versions, URLs, skip-permissions flags), env values never copied, `registry-` kind prefix so a converted entry cannot replace a built-in kind, threat-model row T52.
Also: #813 is still waiting for your fixes.
