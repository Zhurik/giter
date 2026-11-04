# Contributing

## The loop

```sh
cargo test --all
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`make check` runs all of that plus `cargo doc` with warnings denied and the pre-commit hooks. CI runs the same
on every pull request: lints, tests on Linux, macOS and Windows, and a release build of every target the
release ships, so a cross-compile that stops linking shows up before a release does.

## Hooks

The checks are wired up as [pre-commit](https://pre-commit.com) hooks:

```sh
brew install pre-commit        # or: pip install pre-commit
pre-commit install --install-hooks
```

| Stage | Hooks |
|---|---|
| commit | whitespace and line endings, YAML and TOML syntax, `cargo fmt`, `cargo clippy` |
| commit message | the convention below |
| push | `cargo test`, `cargo doc` with warnings denied |

Hooks are a convenience, not a gate: CI runs the same checks, and `git commit --no-verify` is there when you
need it.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org). They are not cosmetic here: releases, version
numbers and the changelog are all derived from them (see [docs/releasing.md](docs/releasing.md)).

```
feat(drawer): sort namespaces by pod count
fix: keep the selection after a refresh
refactor!: rename the repos.json fields
```

Types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `style`, `test`. A `!`
before the colon marks a breaking change.

Pull requests are squash-merged, so it is the **pull request title** that lands on `main`; the `pr-title`
check holds it to the same convention.

## Dependencies

[Renovate](https://docs.renovatebot.com) keeps the crates, the GitHub Actions and the Docker images current
(config in [.github/renovate.json](.github/renovate.json)). Its pull requests merge themselves once every check
is green; one with a red check stays open for a human. Updates to runtime crates are committed as
`fix(deps): …` and therefore produce a patch release, everything else as `chore(deps): …`.
