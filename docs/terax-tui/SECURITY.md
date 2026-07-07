# Terax TUI Security Notes

## Secrets

Prefer environment variables:

```toml
api_key_env = "TERAX_API_KEY"
```

Avoid committing raw API keys.

## Tool Approval

Terax TUI uses explicit approval gates:

- `y/n` for tool execution
- `a/n` for AI edit application

## File Writes

AI generated file changes are staged into the editor buffer first. They are not written to disk until `Ctrl+S`.

## Shell Commands

`terminal_send` is classified by risk and requires approval. Review commands before approving.
