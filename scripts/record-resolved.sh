#!/usr/bin/env bash
# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

# No Cargo.lock is tracked. What cargo resolved for a commit is kept as a git
# note on that commit (refs/notes/resolved), so an old build can be rebuilt:
#   git notes --ref=resolved show <commit>
# prints each lock under a "# ---- <path>" header; write one back to its path
# and build with --locked.
#
#   record: post-commit, attach every lock in the working tree to HEAD.
#   push:   pre-push, send the notes beside the branch. Never blocks a push.
set -u
ref=resolved
case "${1:-}" in
record)
    root=$(git rev-parse --show-toplevel) || exit 0
    cd "$root" || exit 0
    note=$(mktemp)
    # A lock sits beside a tracked manifest; build output is never walked.
    git ls-files -- 'Cargo.toml' '*/Cargo.toml' | sed 's/Cargo\.toml$/Cargo.lock/' | sort |
        while read -r lock; do
        [ -f "$lock" ] || continue
        echo "# ---- $lock"
        cat "$lock"
    done >"$note"
    if [ -s "$note" ]; then
        git notes --ref="$ref" add -f -F "$note" HEAD >/dev/null 2>&1 || echo "record-resolved: could not attach the note" >&2
    fi
    rm -f "$note"
    ;;
push)
    [ -n "${RESOLVED_NOTES_PUSH:-}" ] && exit 0
    # A rebase gives a commit a new id and leaves its note behind.
    git notes --ref="$ref" show HEAD >/dev/null 2>&1 || bash "$0" record
    git rev-parse -q --verify "refs/notes/$ref" >/dev/null || exit 0
    remote=${PRE_COMMIT_REMOTE_NAME:-origin}
    if git fetch -q "$remote" "+refs/notes/$ref:refs/notes/$ref-remote" 2>/dev/null; then
        git notes --ref="$ref" merge -q -s cat_sort_uniq "refs/notes/$ref-remote" 2>/dev/null
    fi
    RESOLVED_NOTES_PUSH=1 git push -q "$remote" "refs/notes/$ref" 2>/dev/null
    exit 0
    ;;
*)
    echo "usage: record-resolved.sh record|push" >&2
    exit 2
    ;;
esac
