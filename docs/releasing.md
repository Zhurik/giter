# Releasing

Nothing is tagged by hand. [release-please](https://github.com/googleapis/release-please) watches `main` and
keeps a pull request titled `chore(main): release X.Y.Z` open, with the version in `Cargo.toml` and
`Cargo.lock` bumped and `CHANGELOG.md` extended. Both are derived from the conventional commits merged since
the last release:

| Commits since the last release | Next version (while below 1.0) |
|---|---|
| a `feat!:` or a `BREAKING CHANGE:` footer | minor: 0.3.0 → 0.4.0 |
| a `feat:` | minor: 0.3.0 → 0.4.0 |
| only `fix:`, `perf:`, `fix(deps):` | patch: 0.3.0 → 0.3.1 |
| only `chore:`, `ci:`, `docs:`, `refactor:`, `test:` | no release |

Once 1.0 is out a breaking change bumps the major version instead.

## Shipping it

Merge the release pull request. The `release` workflow then

1. tags the merge commit `vX.Y.Z` and publishes the GitHub release with the changelog as its notes;
2. builds Linux x86_64 and aarch64 (musl), macOS aarch64 and x86_64, and Windows x86_64, and attaches
   `giter-vX.Y.Z-<target>.tar.gz` (`.zip` on Windows) plus one `SHA256SUMS`;
3. asks [Zhurik/homebrew-tap](https://github.com/Zhurik/homebrew-tap) to re-render `Formula/giter.rb` from that
   release, so `brew upgrade giter` sees it within a minute.

The release pull request is opened with `GITHUB_TOKEN`, which starts no workflows, so the `release` workflow
dispatches CI on its branch itself.

## Secrets

`TAP_DISPATCH_TOKEN` is a fine-grained personal access token scoped to `Zhurik/homebrew-tap` alone, with
`Actions: write` and `Metadata: read`. Without it the release still goes out and step 3 is skipped with a
warning; bump the tap by hand then:

```sh
gh workflow run bump.yml --repo Zhurik/homebrew-tap --field repo=Zhurik/giter --field tag=vX.Y.Z
```

## When something fails

If a build fails on a flake after the release was published, re-run the failed jobs, or run the `release`
workflow by hand with the tag as input: it rebuilds that tag and replaces the archives already attached. If
the tagged code itself does not build, fix it on `main` with a `fix:` commit and ship the next patch release.
