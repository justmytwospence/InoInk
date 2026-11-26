#!/bin/sh
# InoInk installer: downloads the latest release, checks it against the published checksums,
# installs it on a Kobo connected by USB and signs in to your accounts.
#
#   curl -fsSL https://justmytwospence.github.io/InoInk/install.sh | sh
#   curl -fsSL https://justmytwospence.github.io/InoInk/install.sh | sh -s -- [options]
#
# Options:
#   --device MOUNT     the Kobo's drive (default: the usual KOBOeReader mount points)
#   --accounts-only    skip the install: only sign in again or change accounts
#   --service NAME     set up one service only (inoreader or karakeep); repeatable
#   --version X.Y.Z    a release other than the latest
#
# Nothing is installed anywhere except on the Kobo. Everything downloaded is deleted at the end.
# For tests: INOINK_API, INOINK_DOWNLOADS, INOINK_TTY and INOINK_NO_EJECT override the defaults.
set -eu

die() {
	echo "error: $*" >&2
	exit 1
}

main() {
	API=${INOINK_API:-https://api.github.com/repos/justmytwospence/InoInk}
	DOWNLOADS=${INOINK_DOWNLOADS:-https://github.com/justmytwospence/InoInk/releases/download}
	TTY=${INOINK_TTY:-/dev/tty}
	DEVICE=
	ACCOUNTS_ONLY=0
	SERVICES=
	VERSION=
	while [ $# -gt 0 ]; do
		case $1 in
		--device) [ $# -ge 2 ] || die "--device needs a value"; DEVICE=$2; shift 2 ;;
		--accounts-only) ACCOUNTS_ONLY=1; shift ;;
		--service)
			[ $# -ge 2 ] || die "--service needs a value"
			case $2 in *[!a-z]* | '') die "unknown service $2 (inoreader, karakeep)" ;; esac
			SERVICES="$SERVICES --service $2"
			shift 2 ;;
		--version) [ $# -ge 2 ] || die "--version needs a value"; VERSION=$2; shift 2 ;;
		-h | --help) sed -n '2,17p' "$0" 2>/dev/null || true; exit 0 ;;
		*) die "unknown option $1 (see the options at the top of install.sh)" ;;
		esac
	done

	os=$(uname -s)
	arch=$(uname -m)
	case "$os $arch" in
	"Darwin arm64") PLAT=macos-arm64 ;;
	"Linux x86_64") PLAT=linux-x86_64 ;;
	*) die "There is no ready-made inoink-cli for $os $arch. Build it from source (see the README) and run: inoink-cli setup --device <Kobo>" ;;
	esac

	if [ -n "$DEVICE" ]; then
		[ -f "$DEVICE/.kobo/version" ] || die "$DEVICE does not look like a Kobo (no .kobo/version)"
		KOBO=$DEVICE
	else
		KOBO=
		for d in /Volumes/KOBOeReader "/media/${USER:-x}/KOBOeReader" "/run/media/${USER:-x}/KOBOeReader"; do
			if [ -f "$d/.kobo/version" ]; then KOBO=$d; break; fi
		done
		[ -n "$KOBO" ] || die "No Kobo found. Connect it by USB, tap Connect on the Kobo, and run this again (or add --device /path/to/KOBOeReader)."
	fi
	[ -r "$TTY" ] || die "This installer asks questions; run it in a terminal."

	if [ -n "$VERSION" ]; then
		TAG=v${VERSION#v}
	else
		TAG=$(curl -fsSL "$API/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
		[ -n "$TAG" ] || die "Could not find the latest release on GitHub."
	fi
	V=${TAG#v}
	case $V in '' | *[!0-9A-Za-z._-]*) die "bad release name $TAG" ;; esac

	TMP=$(mktemp -d)
	trap 'rm -rf "$TMP"' EXIT
	FILES="inoink-cli-$PLAT"
	[ "$ACCOUNTS_ONLY" = 1 ] || FILES="$FILES install-inoink.sh inoink-$V-kobo.tar.gz"
	echo "Downloading InoInk $V..."
	curl -fsSL -o "$TMP/SHA256SUMS" "$DOWNLOADS/$TAG/SHA256SUMS" || die "Could not download the checksums for $TAG."
	for f in $FILES; do
		curl -fL --progress-bar -o "$TMP/$f" "$DOWNLOADS/$TAG/$f" || die "Could not download $f."
		want=$(awk -v f="$f" '$2 == f || $2 == "*" f { print $1 }' "$TMP/SHA256SUMS")
		if command -v shasum >/dev/null 2>&1; then
			have=$(shasum -a 256 "$TMP/$f" | awk '{ print $1 }')
		elif command -v sha256sum >/dev/null 2>&1; then
			have=$(sha256sum "$TMP/$f" | awk '{ print $1 }')
		else
			die "Need shasum or sha256sum to check the download."
		fi
		if [ -z "$want" ] || [ "$want" != "$have" ]; then
			die "$f does not match the release checksum; nothing was installed."
		fi
	done

	if [ "$ACCOUNTS_ONLY" != 1 ]; then
		INOINK_NO_EJECT=1 sh "$TMP/install-inoink.sh" "$TMP/inoink-$V-kobo.tar.gz" "$KOBO" --menu
	fi
	chmod +x "$TMP/inoink-cli-$PLAT"
	# shellcheck disable=SC2086
	"$TMP/inoink-cli-$PLAT" setup --device "$KOBO" $SERVICES < "$TTY"

	if [ "${INOINK_NO_EJECT:-}" != 1 ] && command -v diskutil >/dev/null 2>&1; then
		if diskutil eject "$KOBO" >/dev/null 2>&1; then echo "The Kobo is ejected."; else echo "Eject the Kobo before unplugging it."; fi
	else
		echo "Eject the Kobo before unplugging it."
	fi
	echo "Unplug the Kobo and open InoInk from NickelMenu: it finishes installing, picks up your accounts and syncs."
}

main "$@"
