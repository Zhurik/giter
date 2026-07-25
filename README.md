# What The Commit (WTC)

![Rust](https://img.shields.io/badge/rust-1.75+-orange)
![License](https://img.shields.io/badge/license-MIT-blue)
![Version](https://img.shields.io/badge/version-0.1.0-green)
![Build Status](https://img.shields.io/github/actions/workflow/status/amzhuravlev/giter/rust.yml?branch=main)
![Tests](https://img.shields.io/github/actions/workflow/status/amzhuravlev/giter/rust.yml?branch=main&label=tests)
![Code Coverage](https://img.shields.io/codecov/c/github/amzhuravlev/giter)

A small utility for checking which commit is currently deployed in your Kubernetes pods.

## Description

This tool helps you quickly verify what git commit is running in your Kubernetes pods. It reads the commit
hash out of each container's image tag and compares it with the tip of the repository's default branch, so
you can see at a glance where a stale build is still running.

## Installation

Build from source:

```bash
cargo build --release
```

## Usage

```bash
giter [OPTIONS]
```

### Options

- `-m, --mode <MODE>` - What a pod is expected to run: `commit` (tip of the default branch, the default) or
  `tag` (the commit behind the latest tag)
- `-n, --namespace <NAMESPACE>` - Show a single namespace instead of every namespace listed in `repos.json`
- `-s, --storage-path <PATH>` - Path to the repos.json file (default: `./repos.json`)
- `-h, --help` - Print help
- `-V, --version` - Print version

### Examples

Every namespace listed in `repos.json`:

```bash
giter
```

A single namespace:

```bash
giter -n micro-1
giter --namespace gateway
```

Production, where every pod is supposed to run the latest tagged release:

```bash
giter --mode tag
```

Specify a custom storage path:

```bash
giter -s /path/to/repos.json
```

## How It Works

1. Takes the namespaces from `repos.json`, or the single one given with `-n`
2. Lists the pods of every namespace in parallel
3. Asks every repository for its target commit in the background — no API token needed, your existing git
   credentials apply
4. Reads the commit hash from each image tag, which CI builds as `<short sha>-<pipeline id>`, and compares it
   with that target

The target depends on the mode:

- `commit` - `git ls-remote <url> HEAD`, the tip of the default branch
- `tag` - `git ls-remote --tags --sort=-v:refname <url>`, the commit behind the highest version tag. Sorting is
  left to `git`, whose version sort puts `v0.1.100` above `v0.1.99` — plain alphabetical would not. Annotated
  tags are followed to the commit they point at

Both lookups are cached per mode, so toggling with `m` only fetches what is missing.

Colours:

- **green** - running the target commit
- **red** - running something else
- **grey** - nothing to compare: a sidecar pinned to a version, a repository without tags, a namespace missing
  from `repos.json`, or a lookup that is still running or failed

### Keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move within the pane |
| `←` `→` / `h` `l` / `Tab` | switch pane |
| `Enter` | walk right through the panes, open the commit in a browser on the last one |
| `m` | switch between `commit` and `tag` mode |
| `r` | re-run the lookup for the selected namespace |
| `q` / `Esc` | quit |

## repos.json Format

`name` must be the **Kubernetes namespace**, not the repository name — that is what the pods are matched
against. `url` is the base project URL without a trailing slash; `/-/commit/<hash>` is appended to it.

```json
[
        {
                "name": "microservice-1",
                "url": "https://my-gitlab.selfhosted.com/some/path/microservice-1"
        },
        {
                "name": "microservice-2",
                "url": "https://my-gitlab.selfhosted.com/some/path/microservice-2"
        }
]
```

## Requirements

- Rust toolchain
- Kubernetes access configured via kubeconfig
- `kubectl` access to the target namespace
- `git` in `PATH`, with read access to the repositories

## Roadmap

- [x] Add tests
- [ ] Add custom rules for specifying commit
- [ ] Optionally use tags
- [ ] Themes
- [ ] CLI mode
- [ ] GitHub support
