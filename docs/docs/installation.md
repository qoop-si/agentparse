# Installation

## Requirements

| Need | Detail |
|---|---|
| Rust | 1.85 or newer, edition 2024. The repository pins the stable toolchain with rustfmt and clippy |
| C compiler | Needed once to build the bundled SQLite that `rusqlite` compiles |
| Platforms | CI runs on Linux, macOS and Windows |

There are no runtime services to start and no network access. agentparse only reads files.

## As a library

The crate is not published yet. Until then, depend on the git repository:

```toml
[dependencies]
agentparse = { git = "https://github.com/qoop-si/agentparse" }
```

Once it is published on crates.io:

```bash
cargo add agentparse
```

If you also want to turn sessions into JSON in your own program, add `serde_json`. The model types implement `Serialize` and `Deserialize`.

## The command line tool

From a checkout of the repository:

```bash
git clone https://github.com/qoop-si/agentparse
cd agentparse
cargo install --path .
```

This installs a binary named `agentparse`. To build without installing:

```bash
cargo build --release
./target/release/agentparse list
```

Running the binary with no arguments (or an unknown command) prints the usage text and exits with status 1.

Check that it works:

```bash
agentparse list
```

`list` prints one line per session it finds in the default locations. An empty result means no session was found there, not an error. See [CLI](#cli) for the commands and [Rust API](#api) for how locations and environment overrides work.

## Dependencies you should know about

| Dependency | Why it matters |
|---|---|
| `rusqlite` with the `bundled` feature | OpenCode stores sessions in SQLite. The C `sqlite3` library is compiled into your build. Cargo allows only one version of that library per build graph, so the dependency is a range, `>=0.32, <0.41`. If your project pins an older `rusqlite` in that range, both resolve to one copy |
| `ruzstd` | Pure Rust decoder for Codex `.jsonl.zst` rollouts |
| `serde`, `serde_json`, `thiserror` | The model derives `Serialize` and `Deserialize`, and `Error` is a `thiserror` enum |

The crate forbids `unsafe` code.

## Where it looks for sessions

`Roots::from_env()` uses each product's default location and the environment variables the products themselves honor:

| Product | Default | Environment override |
|---|---|---|
| Claude Code | `~/.claude` | `CLAUDE_CONFIG_DIR` |
| Codex | `~/.codex` | `CODEX_HOME` |
| OpenCode | `~/.local/share/opencode` | `XDG_DATA_HOME` (uses `$XDG_DATA_HOME/opencode`), `OPENCODE_DB` (an absolute path to one database file, or a file name inside the data directory) |
| Gemini CLI | `~/.gemini` | `GEMINI_CLI_HOME` (uses `$GEMINI_CLI_HOME/.gemini`) |

`~` is `HOME`, or `USERPROFILE` when `HOME` is not set. An empty variable counts as unset.

## Next

- [Quick Start](#quick-start)
