#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The raw release notes of a desktop release — every change between this tag and the
# previous one, one line per squash commit, unedited — written onto the draft release
# the matrix has just built.
#
# ⭐ The shape is the platform repository's `scripts/release-notes.sh`, deliberately and
# exactly: `## What shipped in <version>`, a blank line, then `- <subject>` per commit.
# A reader who has seen one Anthers release page should recognize the other, and the
# release *name* differs only in the product — "Anthers Desktop 2026.10.0" against
# "Anthers 2026.10.37". This is the audit trail, not the reader-facing copy; it is
# never edited to read better.
#
# 🚨 `--no-merges` is load-bearing. Two commits in the shell's history are true
# two-parent merges from before the repo settled on squash merges, and their subjects
# ("Merge pull request #N from …") carry no change information.
#
# The range's base is the previous calver tag reachable from this commit. A tag that is
# not an ancestor is not a previous release of this line, and `--merged HEAD` is what
# keeps a stray or side-branch tag out of the diff. The one release with no previous tag
# is the first, and its range is the whole history: the artifact it ships contains
# everything the shell has ever been, so that is the honest boundary — unlike the
# platform, which can say "production was already running this" because it has a
# `release` branch to diff against.
#
# A re-run of an interrupted promote finds the release already carrying these notes;
# regenerating them converges on the same content, so the edit is idempotent rather
# than an error.
#
# Inputs (environment):
#   GH_TOKEN   the token `gh` authenticates with
#   GH_REPO    owner/name — Actions sets GITHUB_REPOSITORY, which is the default here
#   GITHUB_REF_NAME  the tag, when running in Actions; the tag and package.json's
#                    version are required to agree, which is what the bump PR bought
#
# Everything else it reads itself: the version from package.json, and the commit from
# HEAD.

set -uo pipefail

fail() {
	echo "::error::$1"
	exit 1
}

if [ ! -f package.json ]; then
	fail "run this from the repository root — package.json is not here."
fi
VERSION=$(grep -m1 '"version"' package.json | sed -E 's/.*"version": *"([^"]+)".*/\1/')
printf '%s' "$VERSION" | grep -Eq '^[0-9]{4}\.[0-9]{1,2}\.[0-9]+$' ||
	fail "package.json's version is not calver (got \"${VERSION:-none}\")."

# In Actions the tag is the authority, and a tag disagreeing with the fields is the one
# failure this script exists to refuse: the release would be named one version and
# carry bundles named another.
if [ -n "${GITHUB_REF_NAME:-}" ] && [ "${GITHUB_REF_NAME}" != "v${VERSION}" ]; then
	fail "tag ${GITHUB_REF_NAME} does not match the version ${VERSION} the four fields carry."
fi

REPO="${GH_REPO:-${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set — pass GH_REPO when running by hand}}"

# The previous calver tag, newest by the sort calver orders releases by, reachable from
# this commit and not the release being made.
PREV=$(git tag --merged HEAD --list 'v*' --sort=-version:refname | grep -vx "v${VERSION}" | head -1 || true)
if [ -n "$PREV" ]; then
	git log "${PREV}..HEAD" --no-merges --format='- %s' >release-notes-raw.md
else
	git log HEAD --no-merges --format='- %s' >release-notes-raw.md
fi
[ -s release-notes-raw.md ] || fail "the notes between ${PREV:-the first commit} and HEAD are empty — there is nothing to release."

{
	echo "## What shipped in ${VERSION}"
	echo ""
	cat release-notes-raw.md
} >release-body.md

gh release edit "v${VERSION}" --repo "$REPO" --notes-file release-body.md
echo "::notice::wrote the notes for v${VERSION} (${PREV:-first release}..HEAD, $(wc -l <release-notes-raw.md | tr -d ' ') entries)"
