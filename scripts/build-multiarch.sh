#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: bash scripts/build-multiarch.sh [--arch amd64|arm64|all] [--output directory]

Build Debian trixie packages using Docker Buildx (default: both architectures).
Output defaults to the project's dist/; relative paths use the current directory.
Existing files with the same package name are replaced; other files are untouched.

Options:
  --arch ARCH       amd64, arm64, or all (default: all)
  --output DIR      Destination for flattened .deb artifacts
  -h, --help        Show this help

Environment: DEB_MAINTAINER="Name <email>", DEB_REVISION=1.
Uses your selected Buildx builder; does not install QEMU or create a builder.
See README.md for Docker, QEMU, and docker-container builder setup.
EOF
}
fail() { printf 'Error: %s\n' "$*" >&2; exit 2; }
project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
arch=all
output_dir="$project_dir/dist"
while (( $# )); do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --arch|--output)
            (( $# >= 2 )) && [[ -n $2 && $2 != --* ]] || fail "Missing value for $1. See --help."
            if [[ $1 == --arch ]]; then arch=$2; else output_dir=$2; fi
            shift 2 ;;
        *) fail "Unknown argument: $1. See --help." ;;
    esac
done
case "$arch" in
    all) platforms=linux/amd64,linux/arm64; arches=(amd64 arm64) ;;
    amd64|arm64) platforms="linux/$arch"; arches=("$arch") ;;
    *) fail "Unsupported architecture: $arch. Use amd64, arm64, or all." ;;
esac
revision=${DEB_REVISION:-1}
maintainer=${DEB_MAINTAINER:-Debby contributors <debby@localhost>}
[[ $revision =~ ^[0-9]+$ && $maintainer != *$'\n'* && $maintainer != *$'\r'* ]] || fail 'Invalid DEB_REVISION or DEB_MAINTAINER.'
command -v docker >/dev/null 2>&1 || fail 'Docker is required. See README.md.'
docker buildx version >/dev/null 2>&1 || fail 'Docker Buildx is required. See README.md.'
docker info >/dev/null 2>&1 || fail 'Cannot access the Docker daemon. Check Docker is running and your permissions.'
docker buildx inspect --bootstrap >/dev/null || fail 'Cannot initialize the selected Buildx builder. See README.md.'

export_dir=$(mktemp -d -t debby-export.XXXXXXXX)
trap 'rm -rf -- "$export_dir"' EXIT
docker buildx build --platform "$platforms" \
    --file "$project_dir/Dockerfile" --target artifacts \
    --build-arg "DEB_MAINTAINER=$maintainer" \
    --build-arg "DEB_REVISION=$revision" \
    --output "type=local,dest=$export_dir,platform-split=true" \
    "$project_dir"

# Validate all expected artifacts before copying anything into the user's directory.
packages=()
for architecture in "${arches[@]}"; do
    matches=()
    while IFS= read -r -d '' package; do
        matches+=("$package")
    done < <(find "$export_dir" -type f -name "debby_*_${architecture}.deb" -print0)
    (( ${#matches[@]} == 1 )) || fail "Expected one $architecture package from Buildx, found ${#matches[@]}."
    packages+=("${matches[0]}")
done
mkdir -p -- "$output_dir"
for package in "${packages[@]}"; do
    cp -- "$package" "$output_dir/"
    printf 'Built: %s/%s\n' "$output_dir" "${package##*/}"
done
