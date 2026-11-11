#!/bin/sh
# Downloads every external build input into build/inputs and verifies it against
# build/inputs.lock.json. Fails closed on a mismatch. `--record` writes the lock from what was
# downloaded (use only when deliberately changing a pinned input, and review the diff).
set -eu
cd "$(dirname "$0")/.."
LOCK=build/inputs.lock.json
DEST=build/inputs
mkdir -p "$DEST"
RECORD=0
[ "${1:-}" = "--record" ] && RECORD=1

# name|url
INPUTS='gcc-linaro-4.9.4-2017.01-x86_64_arm-linux-gnueabihf.tar.xz|https://drive.usercontent.google.com/download?id=1xSf7PzfmI2DD7RHsPhwSy-Ltm8gabblK&export=download&confirm=t
zlib-1.3.1.tar.gz|https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz
bzip2-1.0.8.tar.gz|https://sourceware.org/pub/bzip2/bzip2-1.0.8.tar.gz
libpng-1.6.53.tar.gz|https://download.sourceforge.net/libpng/libpng-1.6.53.tar.gz
jpegsrc.v9f.tar.gz|http://www.ijg.org/files/jpegsrc.v9f.tar.gz
openjpeg-2.5.4.tar.gz|https://github.com/uclouvain/openjpeg/archive/v2.5.4.tar.gz
jbig2dec-0.20.tar.gz|https://github.com/ArtifexSoftware/jbig2dec/releases/download/0.20/jbig2dec-0.20.tar.gz
freetype-2.14.1.tar.gz|https://download.savannah.gnu.org/releases/freetype/freetype-2.14.1.tar.gz
harfbuzz-12.3.0.tar.gz|https://github.com/harfbuzz/harfbuzz/archive/12.3.0.tar.gz
gumbo-parser-0.10.1.tar.gz|https://github.com/google/gumbo-parser/archive/v0.10.1.tar.gz
djvulibre-3.5.29.tar.gz|http://downloads.sourceforge.net/djvu/djvulibre-3.5.29.tar.gz
mupdf-1.27.0-source.tar.gz|https://casper.mupdf.com/downloads/archive/mupdf-1.27.0-source.tar.gz'

sha() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 || sha256sum "$1" | cut -d' ' -f1; }

[ -e "$LOCK" ] || echo '{}' > "$LOCK"
NEW=$(cat "$LOCK")
echo "$INPUTS" | while IFS='|' read -r name url; do
	file="$DEST/$name"
	if [ ! -s "$file" ]; then
		echo "Downloading $name"
		curl -fSL --retry 3 -o "$file.part" "$url"
		mv "$file.part" "$file"
	fi
	got=$(sha "$file")
	want=$(jq -r --arg n "$name" '.[$n] // empty' "$LOCK")
	if [ "$RECORD" = 1 ]; then
		NEW=$(echo "$NEW" | jq --arg n "$name" --arg h "$got" '.[$n]=$h')
		echo "$NEW" > "$LOCK.new"
	elif [ -z "$want" ]; then
		echo "error: $name is not pinned in $LOCK (run with --record to pin)" >&2
		exit 1
	elif [ "$want" != "$got" ]; then
		echo "error: $name checksum mismatch: expected $want, got $got" >&2
		exit 1
	else
		echo "ok $name"
	fi
done
if [ "$RECORD" = 1 ]; then
	# The loop ran in a subshell; recompute the lock from the files.
	: > "$LOCK.tmp"
	echo '{}' > "$LOCK.tmp"
	for f in $(echo "$INPUTS" | cut -d'|' -f1); do
		jq --arg n "$f" --arg h "$(sha "$DEST/$f")" '.[$n]=$h' "$LOCK.tmp" > "$LOCK.tmp2" && mv "$LOCK.tmp2" "$LOCK.tmp"
	done
	mv "$LOCK.tmp" "$LOCK"
	rm -f "$LOCK.new"
	echo "Recorded $LOCK"
fi
