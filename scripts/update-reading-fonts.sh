#!/bin/sh
# Refreshes the bundled reading fonts in fonts/reading from pinned, checksummed upstream releases.
# Fonts are redistributed unmodified (no subsetting: Libron reserves its name for the original).
#  - Libron: https://github.com/nicoverbruggen/libron (SIL OFL 1.1)
#  - NV Bitter, NV Jost, NV Literata: https://github.com/nicoverbruggen/ebook-fonts (SIL OFL 1.1)
# To update, change a URL, run once to see the new checksum, check the license strings with
# scripts/license-audit.sh, then pin the checksum here and commit fonts/reading with doc/LICENSES.md.
set -eu
cd "$(dirname "$0")/.."
DEST=fonts/reading
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

sha() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 || sha256sum "$1" | cut -d' ' -f1; }

# name|url|sha256|files to keep
ASSETS='Libron.zip|https://github.com/nicoverbruggen/libron/releases/download/v0.32/Libron.zip|41686314e349fd80326149b9d7d791ffb7cb547a5e8356dae819831b47b1fd9f|Libron-Regular.ttf Libron-Italic.ttf Libron-Bold.ttf Libron-BoldItalic.ttf
other-core-fonts.zip|https://github.com/nicoverbruggen/ebook-fonts/releases/download/v2026.08.05/other-core-fonts.zip|66fd9295e2f22d32772be5c7b1451aac7fdcdcf1d495c761a93448d6de6a854c|NV_Bitter-Regular.ttf NV_Bitter-Italic.ttf NV_Bitter-Bold.ttf NV_Bitter-BoldItalic.ttf NV_Jost-Regular.ttf NV_Jost-Italic.ttf NV_Jost-Bold.ttf NV_Jost-BoldItalic.ttf
other-extra-fonts.zip|https://github.com/nicoverbruggen/ebook-fonts/releases/download/v2026.08.05/other-extra-fonts.zip|ef00c88d57993bf50d87933172a632db1d0c702ed1d163f6c7ea421205f9dee1|NV_Literata-Regular.ttf NV_Literata-Italic.ttf NV_Literata-Bold.ttf NV_Literata-BoldItalic.ttf'

rm -rf "$DEST.new" && mkdir -p "$DEST.new"
echo "$ASSETS" | while IFS='|' read -r name url want files; do
	echo "Downloading $name"
	curl -fsSL --retry 3 -o "$WORK/$name" "$url"
	got=$(sha "$WORK/$name")
	if [ "$got" != "$want" ]; then
		echo "checksum mismatch for $name: got $got, want $want" >&2
		exit 1
	fi
	# shellcheck disable=SC2086 # the file list is meant to split
	unzip -q -j -o "$WORK/$name" $files -d "$DEST.new"
done
chmod 644 "$DEST.new"/*
rm -rf "$DEST" && mv "$DEST.new" "$DEST"
echo "Reading fonts written to $DEST; run scripts/license-audit.sh and review doc/LICENSES.md."
