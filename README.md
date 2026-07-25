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

It opens on an overview of every namespace at once, worst first, and `Enter` takes one apart: its pods, what
each of them runs, and how healthy they are. The header names the cluster you are looking at, because
mistaking production for staging is the easiest mistake to make.

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
- `--colorblind` - Use blue and orange instead of green and red
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
2. Draws the table straight away — the names are known before the cluster has answered anything
3. Lists the pods of every namespace in parallel, filling rows in as they arrive
4. Asks every repository for its target commit in the background — no API token needed, your existing git
   credentials apply
5. Reads the commit hash from each image tag, which CI builds as `<short sha>-<pipeline id>`, and compares it
   with that target

The target depends on the mode:

- `commit` - `git ls-remote <url> HEAD`, the tip of the default branch
- `tag` - `git ls-remote --tags --sort=-v:refname <url>`, the commit behind the highest version tag. Sorting is
  left to `git`, whose version sort puts `v0.1.100` above `v0.1.99` — plain alphabetical would not. Annotated
  tags are followed to the commit they point at

Both lookups are cached per mode, so toggling with `m` only fetches what is missing.

The environment badge comes from the current kubeconfig context: a name containing `prod`, `stag` or `dev`
tints the whole header. Anything else is shown without a badge.

### What a row says

Every row carries a glyph as well as a colour, so it reads in a monochrome terminal and under colour
blindness. When there is nothing to compare, a short code says why — the same four reasons no longer share
one shade of grey.

| Glyph | Meaning | Code | |
| --- | --- | --- | --- |
| `●` green | in sync | | deployed commit equals the reference |
| `▼` red | behind | | deployed commit differs from the reference |
| `✗` orange | failed | `k8s` | cannot list pods — no access or no such namespace |
| `✗` orange | failed | `git` | `ls-remote` failed, the reason is in the row |
| `⠙` grey | resolving | | the reference is still being fetched |
| `○` grey | unknown | `sidecar` | foreign image, such as a service mesh proxy |
| `○` grey | unknown | `digest` | pinned by digest, the tag carries no commit |
| `○` grey | unknown | `no-cfg` | namespace has no entry in `repos.json` |
| `○` grey | unknown | `no-tags` | tag mode, but the repository has no tags |
| `○` grey | unknown | `mode` | a release image, which `commit` mode has no tag list to resolve |
| `○` grey | unknown | `done` | the pod has run its course — what a CronJob leaves behind |
| `○` grey | unknown | `no-pods` | no pods, so nothing to compare |

The DEPLOYED column is named after what it holds: `DEPLOYED COMMIT` where pods run branch builds and
`DEPLOYED TAG` where they run releases. That does not follow the mode — the mode says what pods are compared
against, not what they carry — so a cluster deployed from releases says `TAG` even in `commit` mode.

One lagging pod makes a namespace red, and a namespace is green when every pod that can be judged is. What
cannot be judged is left out rather than dragging everything down with it: a sidecar pinned to its own
version, a pod belonging to somebody else's workload that happens to share the namespace, and a pod that has
already finished — what a CronJob leaves behind ran the code that was current at the time and deploys nothing
now — say nothing about the release and are not counted as deployed commits either. Only when nothing at all can be judged does
the namespace take on the reason why. The DEPLOYED COMMIT column lists every distinct commit its own pods run
with a count, so a rollout that stopped half way through shows up as two groups.

In the overview the prefix a namespace shares with its family is dimmed rather than cut, and it is the prefix
that gives up room first when the column runs out. The list on the namespace screen goes further: it groups
namespaces under a heading carrying that whole shared prefix, so the rows below show only what tells them
apart.

Below 100 columns the table drops to name, commit and pod count; below 60×15 it says so instead of drawing
something unreadable.

### Keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move |
| `PgUp` `PgDn` / `Home` `End` | jump ten rows / to either end |
| `Enter` | open the namespace, or open the deployed commit in a browser |
| `←` `→` / `h` `l` / `Tab` | switch pane, or step back to the overview |
| `Esc` | dismiss a message, leave the namespace, quit |
| `/` | search namespaces |
| `f` | filter: all / problems / unknown |
| `s` | sort: status / name / pods |
| `m` | switch between `commit` and `tag` mode |
| `r` | re-resolve the selected namespace |
| `R` | re-list the pods from Kubernetes |
| `?` | full key map and legend |
| `q` | quit |

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
- [x] Optionally use tags
- [ ] Add custom rules for specifying commit
- [ ] Themes
- [ ] CLI mode
- [ ] GitHub support
