# What The Commit (WTC)

![Rust](https://img.shields.io/badge/rust-1.75+-orange)
![License](https://img.shields.io/badge/license-MIT-blue)
![Version](https://img.shields.io/badge/version-0.1.0-green)
![Build Status](https://img.shields.io/github/actions/workflow/status/amzhuravlev/giter/rust.yml?branch=main)
![Tests](https://img.shields.io/github/actions/workflow/status/amzhuravlev/giter/rust.yml?branch=main&label=tests)
![Code Coverage](https://img.shields.io/codecov/c/github/amzhuravlev/giter)

A small utility for checking which commit is currently deployed in your Kubernetes pods.

## Description

This tool helps you quickly verify what git commit is running in your Kubernetes pods by inspecting pod labels and cross-referencing them with your local git repositories.

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

- `-n, --namespace <NAMESPACE>` - Kubernetes namespace to check pods in. If not provided, uses the current namespace from your kubeconfig
- `-s, --storage-path <PATH>` - Path to the repos.json file (default: `./repos.json`)
- `-h, --help` - Print help
- `-V, --version` - Print version

### Examples

Use current namespace from kubeconfig:

```bash
giter
```

Specify a namespace:

```bash
giter -n micro-1
giter --namespace gateway
```

Specify a custom storage path:

```bash
giter -s /path/to/repos.json
```

## How It Works

1. Reads the current namespace from kubeconfig or uses the `-n` flag
2. Fetches all pods in the specified namespace
3. Displays pod information with commit hashes
4. Cross-references commits with local git repositories from `repos.json`

## repos.json Format

The storage file contains mappings of repository paths to their commit hashes:

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

## Roadmap

- [ ] Add tests
- [ ] Add custom rules for specifying commit
- [ ] Optionally use tags
- [ ] Themes
- [ ] CLI mode
- [ ] GitHub support
