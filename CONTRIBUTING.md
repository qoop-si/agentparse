# Contributing

Thanks for helping. Issues and pull requests are welcome.

## Before you open a pull request

Run the same checks as CI:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Add a test for a new transcript shape. Test fixtures must be synthetic: no real prompts, code, paths, usernames or hostnames.

## How pull requests land

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:` and so on). Releases and the changelog are generated from them.
- Pull requests are merged by rebase, so keep each commit meaningful on its own.

## License

By contributing you agree that your contribution is licensed under the MIT license of this repository.
