#!/bin/sh
# Rebuild NotoSansSC-UI.otf: Noto Sans CJK SC Regular cut down to the
# non-ASCII characters in ../../src, plus CJK punctuation.
# Rerun after adding Chinese text to the window; `cargo test -p usb-i2c-gui`
# fails until the subset covers it.
#
# Needs fonttools (pip install fonttools) and the source font, either
#   SRC=.../NotoSansCJKsc-Regular.otf FONT_NUMBER=0 ./subset.sh
# or the Debian/Ubuntu collection (fonts-noto-cjk), the default:
#   ./subset.sh
set -eu
cd "$(dirname "$0")"
SRC=${SRC:-/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc}
# index of "Noto Sans CJK SC" in NotoSansCJK-Regular.ttc; 0 for a single .otf
FONT_NUMBER=${FONT_NUMBER:-2}

chars=$(mktemp)
trap 'rm -f "$chars"' EXIT
find ../../src -name '*.rs' -exec cat {} + |
    python3 -I -c 'import sys
s = set(c for c in sys.stdin.read() if ord(c) > 127)
s |= set("，。、：；！？（）「」『』《》【】—…·　")
sys.stdout.write("".join(sorted(s)))' >"$chars"

pyftsubset "$SRC" \
    --font-number="$FONT_NUMBER" \
    --text-file="$chars" \
    --layout-features='kern,vert,locl' \
    --output-file=NotoSansSC-UI.otf
echo "NotoSansSC-UI.otf: $(wc -c <NotoSansSC-UI.otf) bytes, $(wc -m <"$chars") characters"
