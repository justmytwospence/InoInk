#!/bin/sh
# Builds Plato's native dependencies for the Kobo (ARM, glibc, hard-float) from the pinned source
# archives, using the toolchain on PATH. Runs inside the build container (see build/Containerfile).
#   $1 = directory holding the verified source archives
#   $2 = directory to place the shared libraries in
set -eu
INPUTS=$1
OUT=$2
cd /opt/kobo/thirdparty

extract() { # dir archive
	rm -rf "$1/.git" 2>/dev/null || true
	tar -xzf "$INPUTS/$2" --strip-components 1 -C "$1"
}

extract zlib zlib-1.3.1.tar.gz
extract bzip2 bzip2-1.0.8.tar.gz
extract libpng libpng-1.6.53.tar.gz
extract libjpeg jpegsrc.v9f.tar.gz
extract openjpeg openjpeg-2.5.4.tar.gz
extract jbig2dec jbig2dec-0.20.tar.gz
extract freetype2 freetype-2.14.1.tar.gz
extract harfbuzz harfbuzz-12.3.0.tar.gz
extract gumbo gumbo-parser-0.10.1.tar.gz
extract djvulibre djvulibre-3.5.29.tar.gz
extract mupdf mupdf-1.27.0-source.tar.gz

for name in zlib bzip2 libpng libjpeg openjpeg jbig2dec freetype2 harfbuzz gumbo djvulibre mupdf; do
	echo "Building $name"
	(
		cd "$name"
		[ -e kobo.patch ] && patch -p 1 < kobo.patch
		./build-kobo.sh
	)
done

mkdir -p "$OUT"
cp zlib/libz.so "$OUT"
cp bzip2/libbz2.so "$OUT"
cp libpng/.libs/libpng16.so "$OUT"
cp libjpeg/.libs/libjpeg.so "$OUT"
cp openjpeg/build/bin/libopenjp2.so "$OUT"
cp jbig2dec/.libs/libjbig2dec.so "$OUT"
cp freetype2/objs/.libs/libfreetype.so "$OUT"
cp harfbuzz/build/src/libharfbuzz.so "$OUT"
cp gumbo/.libs/libgumbo.so "$OUT"
cp djvulibre/libdjvu/.libs/libdjvulibre.so "$OUT"
cp mupdf/build/release/libmupdf.so "$OUT"
