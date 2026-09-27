#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPOSITORY_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
OUTPUT_DIRECTORY=${1:-"$REPOSITORY_ROOT/dist"}
PACKAGER="$REPOSITORY_ROOT/scripts/package-macos-arm64.sh"

archive_paths=$(find "$OUTPUT_DIRECTORY" -maxdepth 1 -type f \
    -name 'echidna-*-aarch64-apple-darwin.tar.gz' -print)
archive_count=$(printf '%s\n' "$archive_paths" | awk 'NF { count += 1 } END { print count + 0 }')
test "$archive_count" -eq 1 || {
    echo "distribution regression test requires exactly one release archive" >&2
    exit 1
}

archive_path=$archive_paths
archive_name=$(basename "$archive_path")
package_directory=${archive_name%.tar.gz}

"$PACKAGER" --verify-only "$OUTPUT_DIRECTORY"

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/echidna-distribution-test.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT HUP INT TERM

expect_rejected() {
    fixture_directory=$1
    failure_message=$2
    if "$PACKAGER" --verify-only "$fixture_directory" >/dev/null 2>&1; then
        echo "$failure_message" >&2
        exit 1
    fi
}

checksum_fixture="$temporary_root/checksum"
mkdir "$checksum_fixture"
cp "$archive_path" "$OUTPUT_DIRECTORY/SHA256SUMS" "$checksum_fixture/"
perl -pi -e 's/^[0-9a-f]/x/' "$checksum_fixture/SHA256SUMS"
expect_rejected "$checksum_fixture" "a modified SHA256SUMS was accepted"

log_fixture="$temporary_root/log"
log_stage="$log_fixture/stage"
mkdir -p "$log_stage"
tar -xzf "$archive_path" -C "$log_stage"
touch "$log_stage/$package_directory/local.log"
COPYFILE_DISABLE=1 tar -czf "$log_fixture/$archive_name" \
    -C "$log_stage" "$package_directory"
(CDPATH= cd -- "$log_fixture" && shasum -a 256 "$archive_name" > SHA256SUMS)
expect_rejected "$log_fixture" "a checksum-valid archive containing a local log was accepted"

link_fixture="$temporary_root/link"
link_stage="$link_fixture/stage"
mkdir -p "$link_stage/$package_directory"
ln -s /usr/bin/true "$link_stage/$package_directory/echidna"
COPYFILE_DISABLE=1 tar -czf "$link_fixture/$archive_name" \
    -C "$link_stage" "$package_directory"
(CDPATH= cd -- "$link_fixture" && shasum -a 256 "$archive_name" > SHA256SUMS)
expect_rejected "$link_fixture" "a checksum-valid archive containing a binary symlink was accepted"

printf '%s\n' "distribution archive negative checks passed"
