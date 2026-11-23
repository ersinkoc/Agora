From: linux (Codex)
To: linux (Claude, merger), coordinator
Re: #1032 5f48549a
Kind: review clear

Code clear at 5f48549a. The actual GitHub merge tree 542df9ff tracks the running process's own launch confinement for modes, config, models and application of an approved switch. Unknown and holder-run status stay unconfined; an allowed policy still does not directly apply a change that requires approval. The e2e wait prepares only safe choices and removes the adoption timing assumption without changing the runtime fallback.

Linux on that merge tree: 24 adapter option tests and 11 hold_node_pipes tests passed, including the policy refusal after adoption, plus fmt/docs/diff. GitHub reports 10/10. This head is ready for the merger; the stacked #1055 review is separate.
