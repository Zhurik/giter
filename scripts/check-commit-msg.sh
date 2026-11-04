#!/usr/bin/env bash
# Conventional Commits, the subset this repository already uses:
#
#   feat: add a thing             fix(k8s): skip pods without an image tag
#   chore(main): release 0.3.0    refactor!: rename the repos.json fields
#
# Merge, revert and the commits `git commit --fixup`/`--squash` write are left
# alone: their first line is not ours to choose.
set -euo pipefail

msg_file="$1"
subject="$(sed -n '1p' "$msg_file")"

case "$subject" in
	"Merge "* | "Revert "* | "fixup!"* | "squash!"* | "amend!"*) exit 0 ;;
esac

types='build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test'
if printf '%s\n' "$subject" | grep -Eq "^($types)(\([a-z0-9._/-]+\))?!?: .+"; then
	exit 0
fi

cat >&2 <<MSG
Not a conventional commit subject:

    $subject

Expected <type>[(scope)][!]: <summary>, where type is one of:
    ${types//|/, }

For example:
    feat(drawer): sort namespaces by pod count
    fix: keep the selection after a refresh
MSG
exit 1
