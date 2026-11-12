#!/bin/sh
# Prints an InoInk version derived from git. The latest v* tag is the latest release.
#   version.sh dev    a dev build: <last release>-dev.<commits since>.<commit>[.dirty]
#   version.sh next   the release that merging HEAD should publish, from the conventional-commit
#                     subjects since the last release: a breaking change (type! or
#                     "BREAKING CHANGE:") bumps major, feat bumps minor, anything else patch.
#                     Commits that touch only docs, CI or tests do not count; prints nothing when
#                     no commit since the last release changes what ships.
# Needs full history and tags (actions/checkout with fetch-depth: 0).
set -eu
cd "$(dirname "$0")/.."

LAST=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
BASE=${LAST#v}
[ -n "$BASE" ] || BASE=0.0.0
RANGE=${LAST:+$LAST..}HEAD

# Paths that never reach a user.
NOT_SHIPPED='^(doc/|site/|[^/]*\.md$|LICENSE|THIRD-PARTY-NOTICES$|\.github/|\.agents/|\.claude/|skills-lock\.json$|tests/|crates/[^/]*/tests/|scripts/(version|kobo-dev|test-inoink|check-reproducibility|license-audit)\.sh$)'

case "${1:-}" in
	dev)
		N=$(git rev-list --count "$RANGE")
		SHA=$(git rev-parse --short=7 HEAD)
		DIRTY=""
		[ -z "$(git status --porcelain --untracked-files=no)" ] || DIRTY=.dirty
		echo "$BASE-dev.$N.$SHA$DIRTY"
		;;
	next)
		BUMP=0 # 0 none, 1 patch, 2 minor, 3 major
		for c in $(git rev-list --no-merges "$RANGE"); do
			git diff-tree --no-commit-id --name-only -r "$c" | grep -qvE "$NOT_SHIPPED" || continue
			SUBJECT=$(git log -1 --format=%s "$c")
			B=1
			if echo "$SUBJECT" | grep -qE '^[a-z]+(\([^)]*\))?!:' || git log -1 --format=%b "$c" | grep -q '^BREAKING CHANGE:'; then
				B=3
			elif echo "$SUBJECT" | grep -qE '^feat(\([^)]*\))?:'; then
				B=2
			fi
			[ "$B" -gt "$BUMP" ] && BUMP=$B
		done
		[ "$BUMP" -gt 0 ] || exit 0
		IFS=. read -r MA MI PA <<-EOF
			$BASE
		EOF
		case $BUMP in
			3) echo "$((MA + 1)).0.0" ;;
			2) echo "$MA.$((MI + 1)).0" ;;
			1) echo "$MA.$MI.$((PA + 1))" ;;
		esac
		;;
	*)
		echo "usage: $0 dev|next" >&2
		exit 2
		;;
esac
