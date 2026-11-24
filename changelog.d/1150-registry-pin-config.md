- An agent from the ACP registry that starts through `npx` or `uvx` now starts its package runner in an empty folder of its own
  instead of the project, so a project's `.npmrc`, `package.json`, `node_modules`, `uv.toml`, `pyproject.toml` or `.python-version`
  can no longer change which package the pinned version resolves to; your own npm and uv settings (a mirror, a proxy, a registry in
  `~/.npmrc` or your user uv configuration) still apply, and the agent still works in the project (#1150, ADR-0092)
