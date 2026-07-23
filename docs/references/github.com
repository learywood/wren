# GitHub references

Repositories that may be useful for Wren implementation research:

- [can1357/oh-my-pi](https://github.com/can1357/oh-my-pi) — Terminal coding agent with an optimized tool harness, LSP support, browser tools, and subagents.
- [agentclientprotocol/agent-client-protocol](https://github.com/agentclientprotocol/agent-client-protocol) — Protocol for connecting editors to agents.
- [0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig) — Modular Rust framework for building LLM applications.
- [Dicklesworthstone/pi_agent_rust](https://github.com/Dicklesworthstone/pi_agent_rust) — Rust implementation of a high-performance coding agent CLI.
- [fortunto2/rust-code](https://github.com/fortunto2/rust-code) — Rust terminal coding agent with a TUI, skills, MCP support, and background tasks.
- [vinhnx/VTCode](https://github.com/vinhnx/VTCode) — Rust coding agent with native sandboxing and multi-provider support.

## Sync location

Keep reference checkouts outside the Wren repository so they do not affect project file search. Use:

```text
${WREN_REFERENCES_DIR:-$HOME/.wren/references}/github.com/<owner>/<repo>
```

`WREN_REFERENCES_DIR` is optional. The default is machine-agnostic because it is relative to the current user's home directory; set the variable when references need to live elsewhere.

Clone a repository with:

```sh
reference_root="${WREN_REFERENCES_DIR:-$HOME/.wren/references}/github.com"
repo="can1357/oh-my-pi"
mkdir -p "$reference_root/$(dirname "$repo")"
git clone "https://github.com/$repo.git" "$reference_root/$repo"
```

Sync an existing checkout with:

```sh
reference_root="${WREN_REFERENCES_DIR:-$HOME/.wren/references}/github.com"
repo="can1357/oh-my-pi"
git -C "$reference_root/$repo" pull --ff-only
```
