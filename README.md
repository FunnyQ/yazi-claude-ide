# yazi-claude-ide

Yazi plugin that speaks Claude Code's `/ide` protocol, so Claude Code can pull context (currently focused/selected file) from [yazi](https://yazi-rs.github.io/) the same way it does from VS Code or Neovim.

Status: Discovery. See [PLAN.md](PLAN.md) for scope, open questions, and task breakdown.

## Development

```sh
bun install
bun test                        # contract tests; each names the clause it covers
bun run typecheck               # tsc --noEmit; bun test is runtime-only
test/manual/harness.sh verify   # the clauses only a real yazi can show
```

[docs/contract.md](docs/contract.md) is the specification.
