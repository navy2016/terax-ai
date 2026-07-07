# Terax TUI

Terminal-first AI-native development workspace for Alpine Linux musl / aarch64.

## Install

Use the musl/aarch64 artifact and place `terax-tui` on PATH:

```bash
cp terax-tui /usr/local/bin/terax-tui
chmod +x /usr/local/bin/terax-tui
```

## Run

```bash
terax-tui
terax-tui --doctor
terax-tui --version
terax-tui --help
```

## Config

`~/.config/terax-tui/config.toml`:

```toml
base_url = "https://example.com/v1"
api_key_env = "TERAX_API_KEY"
model = "gpt-5.5"
```

`api_key_env` is preferred over `api_key`.

## Panels

- `Ctrl+A` AI chat
- `Ctrl+G` Git
- `Ctrl+T` Terminal
- `Ctrl+E` Editor
- `Ctrl+D` Diff preview
- `Ctrl+H` Help

## Commands

- `ai <prompt>`
- `agent <task>`
- `run <cmd>` with approval
- `ai-edit-line <instruction>`
- `ai-edit-buffer <instruction>`
- `ai-write-file <path> <instruction>`
- `/project-root`, `/index`, `/memory`, `/init-repo`
- `/review-diff`, `/commit-message`, `/terminal-tail`, `/git-status`

## Safety

Tools and edits require approval:

- `y/n` for tools
- `a/n` for edits

AI write/edit operations do not write to disk directly. They apply to editor buffer first; use `Ctrl+S` to save.
