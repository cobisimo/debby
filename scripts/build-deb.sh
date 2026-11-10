#!/usr/bin/env bash
set -euo pipefail

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
    echo 'Usage: bash scripts/build-deb.sh [output-directory]'
    echo 'Builds a native release binary and a Debian package (default: dist/).'
    echo 'Optional environment: DEB_MAINTAINER="Name <email>", DEB_REVISION=1'
    exit 0
fi
if (( $# > 1 )); then
    echo 'Expected at most one output directory. See --help.' >&2
    exit 2
fi

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"
for tool in cargo rustc python3 blueprint-compiler msgfmt dpkg-deb dpkg-shlibdeps dpkg-architecture; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "Missing build dependency: $tool. See Readme.md." >&2
        exit 1
    fi
done

output_dir=${1:-dist}
mkdir -p -- "$output_dir"
output_dir=$(cd -- "$output_dir" && pwd)
revision=${DEB_REVISION:-1}
maintainer=${DEB_MAINTAINER:-Debby contributors <debby@localhost>}
if [[ ! $revision =~ ^[0-9]+$ || $maintainer == *$'\n'* || $maintainer == *$'\r'* ]]; then
    echo 'Invalid DEB_REVISION or DEB_MAINTAINER.' >&2
    exit 2
fi

# Cargo metadata supplies the actual version and target directory, including
# custom CARGO_TARGET_DIR settings. This builder packages the native host only.
metadata=$(cargo metadata --no-deps --format-version 1)
version=$(python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "debby"))' <<< "$metadata")
target_dir=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' <<< "$metadata")
host=$(rustc -vV | sed -n 's/^host: //p')
architecture=$(dpkg-architecture -qDEB_BUILD_ARCH)
DEBBY_INSTALL_LOCALEDIR=/usr/share/locale cargo build --release --target "$host" --bin debby
binary="$target_dir/$host/release/debby"

package_tmp=$(mktemp -d -t debby-deb.XXXXXXXX)
trap 'rm -rf -- "$package_tmp"' EXIT
stage="$package_tmp/package"
install -Dm755 "$binary" "$stage/usr/bin/debby"
install -Dm644 com.cobisimo.Debby.desktop "$stage/usr/share/applications/com.cobisimo.Debby.desktop"
install -Dm644 data/icons/hicolor/scalable/apps/com.cobisimo.Debby.svg "$stage/usr/share/icons/hicolor/scalable/apps/com.cobisimo.Debby.svg"
while IFS= read -r language || [[ -n $language ]]; do
    [[ -z $language || $language == \#* ]] && continue
    if [[ ! $language =~ ^[A-Za-z0-9_@.-]+$ ]]; then
        echo "Invalid locale in po/LINGUAS: $language" >&2
        exit 1
    fi
    mkdir -p "$stage/usr/share/locale/$language/LC_MESSAGES"
    msgfmt --check "po/$language.po" -o "$stage/usr/share/locale/$language/LC_MESSAGES/debby.mo"
    chmod 644 "$stage/usr/share/locale/$language/LC_MESSAGES/debby.mo"
done < po/LINGUAS

# dpkg-shlibdeps requires package metadata in the current directory.
mkdir -p "$package_tmp/debian" "$stage/DEBIAN"
printf 'Source: debby\nSection: admin\nPriority: optional\nMaintainer: %s\nStandards-Version: 4.6.2\n\nPackage: debby\nArchitecture: any\nDescription: Install local Debian packages\n' "$maintainer" > "$package_tmp/debian/control"
shlibs=$(cd "$package_tmp" && dpkg-shlibdeps -O -e"$stage/usr/bin/debby")
shlibs=${shlibs#shlibs:Depends=}
if [[ -z $shlibs || $shlibs == *$'\n'* ]]; then
    echo 'Could not determine shared library dependencies.' >&2
    exit 1
fi
installed_size=$(du -sk "$stage/usr" | cut -f1)
{
    printf 'Package: debby\nVersion: %s-%s\nArchitecture: %s\n' "$version" "$revision" "$architecture"
    printf 'Maintainer: %s\nSection: admin\nPriority: optional\n' "$maintainer"
    printf 'Installed-Size: %s\n' "$installed_size"
    printf 'Depends: %s, libadwaita-1-0 (>= 1.5), apt, dpkg, pkexec\n' "$shlibs"
    printf 'Description: Install and manage local Debian packages\n'
    printf ' Debby provides a GTK and libadwaita interface to inspect, install,\n'
    printf ' upgrade, reinstall, and uninstall local Debian packages, with\n'
    printf ' version comparisons, progress reporting, and command output.\n'
} > "$stage/DEBIAN/control"
find "$stage" -type d -exec chmod 755 {} +
package_file="$output_dir/debby_${version}-${revision}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$stage" "$package_file"
dpkg-deb --info "$package_file"
echo "Built: $package_file"
