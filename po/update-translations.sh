#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
translation_tmp=$(mktemp -d)
trap 'rm -rf "$translation_tmp"' EXIT HUP INT TERM
blueprint-compiler compile --output "$translation_tmp/window.ui" data/window.blp
xgettext --language=Rust --from-code=UTF-8 --keyword=tr --package-name=debby --package-version=0.1.0 --copyright-holder='Debby contributors' -o "$translation_tmp/rust.pot" src/main.rs
xgettext --language=Glade --from-code=UTF-8 --no-location -o "$translation_tmp/ui.pot" "$translation_tmp/window.ui"
msgcat --use-first "$translation_tmp/rust.pot" "$translation_tmp/ui.pot" -o po/debby.pot
while read -r language; do
    case "$language" in ''|\#*) continue ;; esac
    msgmerge --update --no-fuzzy-matching "po/$language.po" po/debby.pot
done < po/LINGUAS
