#!/usr/bin/env bash
# Downloads the full Noto faces this harness registers. The test suite keeps
# its own subsets under mutil-engine/tests/assets; these are the complete
# fonts so the page can take arbitrary text in the scripts it shows.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p fonts
base="https://raw.githubusercontent.com/google/fonts/main/ofl"

fetch() {
  local dir="$1" file="$2" out="$3"
  local url="$base/$dir/$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$file")"
  if [ -s "fonts/$out" ]; then
    echo "have $out"
    return
  fi
  echo "get  $out"
  curl -fL --retry 3 --silent --show-error -o "fonts/$out.part" "$url"
  mv "fonts/$out.part" "fonts/$out"
}

fetch notosans 'NotoSans[wdth,wght].ttf' 'NotoSans[wdth,wght].ttf'
fetch notoserif 'NotoSerif[wdth,wght].ttf' 'NotoSerif[wdth,wght].ttf'
fetch notosanssc 'NotoSansSC[wght].ttf' 'NotoSansSC[wght].ttf'
fetch notosanstc 'NotoSansTC[wght].ttf' 'NotoSansTC[wght].ttf'
fetch jetbrainsmono 'JetBrainsMono[wght].ttf' 'JetBrainsMono[wght].ttf'
fetch notosansjp 'NotoSansJP[wght].ttf' 'NotoSansJP[wght].ttf'
fetch notosansdevanagari 'NotoSansDevanagari[wdth,wght].ttf' 'NotoSansDevanagari[wdth,wght].ttf'
fetch notosansthai 'NotoSansThai[wdth,wght].ttf' 'NotoSansThai[wdth,wght].ttf'
fetch notosanslao 'NotoSansLao[wdth,wght].ttf' 'NotoSansLao[wdth,wght].ttf'
fetch notosanskhmer 'NotoSansKhmer[wdth,wght].ttf' 'NotoSansKhmer[wdth,wght].ttf'
fetch notosansmyanmar 'NotoSansMyanmar[wdth,wght].ttf' 'NotoSansMyanmar[wdth,wght].ttf'
fetch notosanshebrew 'NotoSansHebrew[wdth,wght].ttf' 'NotoSansHebrew[wdth,wght].ttf'
fetch notonaskharabic 'NotoNaskhArabic[wght].ttf' 'NotoNaskhArabic[wght].ttf'
fetch notosans 'OFL.txt' 'OFL.txt'

if command -v shasum >/dev/null; then
  shasum -a 256 -c fonts.sha256
elif command -v sha256sum >/dev/null; then
  sha256sum -c fonts.sha256
fi
echo "fonts ready in demo/fonts"
