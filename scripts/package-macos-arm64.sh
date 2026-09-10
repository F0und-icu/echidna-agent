#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPOSITORY_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
TARGET_TRIPLE=aarch64-apple-darwin
BINARY_NAME=cyber-agent

usage() {
    cat >&2 <<'EOF'
usage: package-macos-arm64.sh [OUTPUT_DIRECTORY]
       package-macos-arm64.sh --print-version
       package-macos-arm64.sh --verify-only [OUTPUT_DIRECTORY]
EOF
}

package_version() {
    package_id=$(cargo pkgid --locked --manifest-path "$REPOSITORY_ROOT/Cargo.toml")
    case "$package_id" in
        *'#'*) version=${package_id##*#} ;;
        *)
            echo "cargo pkgid returned an unsupported package identity: $package_id" >&2
            exit 1
            ;;
    esac
    version=${version##*@}
    case "$version" in
        '' | *[!0-9A-Za-z.+-]*)
            echo "cargo pkgid returned an invalid package version: $version" >&2
            exit 1
            ;;
    esac
    printf '%s\n' "$version"
}

resolve_output_directory() {
    requested=${1:-dist}
    case "$requested" in
        /*) printf '%s\n' "$requested" ;;
        *) printf '%s/%s\n' "$REPOSITORY_ROOT" "$requested" ;;
    esac
}

verify_distribution() {
    output_directory=$1
    version=$2
    package_directory="$BINARY_NAME-$version-$TARGET_TRIPLE"
    archive_name="$package_directory.tar.gz"
    archive_path="$output_directory/$archive_name"
    checksum_path="$output_directory/SHA256SUMS"

    test -f "$archive_path" || {
        echo "release archive is missing: $archive_path" >&2
        exit 1
    }
    test -f "$checksum_path" || {
        echo "release checksum is missing: $checksum_path" >&2
        exit 1
    }

    checksum_entries=$(awk 'NF { count += 1 } END { print count + 0 }' "$checksum_path")
    test "$checksum_entries" -eq 1 || {
        echo "SHA256SUMS must describe exactly one release archive" >&2
        exit 1
    }
    checksum_archive=$(awk 'NF { print $2 }' "$checksum_path")
    checksum_archive=${checksum_archive#\*}
    test "$checksum_archive" = "$archive_name" || {
        echo "SHA256SUMS references an unexpected file: $checksum_archive" >&2
        exit 1
    }
    (CDPATH= cd -- "$output_directory" && shasum -a 256 -c SHA256SUMS)

    archive_entries=$(tar -tzf "$archive_path")
    expected_entries=$(printf '%s/\n%s/%s' \
        "$package_directory" \
        "$package_directory" \
        "$BINARY_NAME")
    test "$archive_entries" = "$expected_entries" || {
        echo "release archive contains files outside the binary-only allowlist" >&2
        printf '%s\n' "$archive_entries" >&2
        exit 1
    }
    archive_entry_types=$(tar -tvzf "$archive_path" | awk '{ print substr($1, 1, 1) }')
    expected_entry_types=$(printf 'd\n-')
    test "$archive_entry_types" = "$expected_entry_types" || {
        echo "release archive entries must be one directory and one regular file" >&2
        tar -tvzf "$archive_path" >&2
        exit 1
    }

    verification_root=$(mktemp -d "${TMPDIR:-/tmp}/cyber-agent-verify.XXXXXX")
    trap 'rm -rf "$verification_root"' EXIT HUP INT TERM
    tar -xzf "$archive_path" -C "$verification_root"
    packaged_binary="$verification_root/$package_directory/$BINARY_NAME"
    test -d "$verification_root/$package_directory" \
        && test ! -L "$verification_root/$package_directory" || {
        echo "packaged top-level entry is not a real directory" >&2
        exit 1
    }
    test -f "$packaged_binary" && test ! -L "$packaged_binary" || {
        echo "packaged binary is not a regular file" >&2
        exit 1
    }
    test -x "$packaged_binary" || {
        echo "packaged binary is not executable" >&2
        exit 1
    }
    file "$packaged_binary" | grep -Eq 'Mach-O 64-bit executable arm64' || {
        echo "packaged binary is not a macOS arm64 executable" >&2
        file "$packaged_binary" >&2
        exit 1
    }
    case "$(uname -m)" in
        arm64 | aarch64)
            reported_version=$("$packaged_binary" --version)
            test "$reported_version" = "$BINARY_NAME $version" || {
                echo "packaged binary version does not match Cargo metadata" >&2
                exit 1
            }
            ;;
    esac
    rm -rf "$verification_root"
    trap - EXIT HUP INT TERM
}

mode=package
case "${1:-}" in
    --print-version)
        test "$#" -eq 1 || {
            usage
            exit 2
        }
        package_version
        exit 0
        ;;
    --verify-only)
        mode=verify
        shift
        ;;
    -*)
        usage
        exit 2
        ;;
esac

test "$#" -le 1 || {
    usage
    exit 2
}

cd "$REPOSITORY_ROOT"
version=$(package_version)
output_directory=$(resolve_output_directory "${1:-dist}")
package_directory="$BINARY_NAME-$version-$TARGET_TRIPLE"
archive_name="$package_directory.tar.gz"
archive_path="$output_directory/$archive_name"
checksum_path="$output_directory/SHA256SUMS"

if test "$mode" = verify; then
    verify_distribution "$output_directory" "$version"
    exit 0
fi

cargo build --locked --release --target "$TARGET_TRIPLE"

target_directory=${CARGO_TARGET_DIR:-target}
case "$target_directory" in
    /*) ;;
    *) target_directory="$REPOSITORY_ROOT/$target_directory" ;;
esac
source_binary="$target_directory/$TARGET_TRIPLE/release/$BINARY_NAME"
test -x "$source_binary" || {
    echo "release binary is missing after the locked build: $source_binary" >&2
    exit 1
}

mkdir -p "$output_directory"
rm -f "$archive_path" "$checksum_path"

staging_root=$(mktemp -d "${TMPDIR:-/tmp}/cyber-agent-package.XXXXXX")
trap 'rm -rf "$staging_root"' EXIT HUP INT TERM
mkdir "$staging_root/$package_directory"
cp "$source_binary" "$staging_root/$package_directory/$BINARY_NAME"
chmod 0755 "$staging_root/$package_directory/$BINARY_NAME"

(CDPATH= cd -- "$staging_root" && COPYFILE_DISABLE=1 tar -czf "$archive_path" "$package_directory")
(CDPATH= cd -- "$output_directory" && shasum -a 256 "$archive_name" > SHA256SUMS)

rm -rf "$staging_root"
trap - EXIT HUP INT TERM
verify_distribution "$output_directory" "$version"

printf '%s\n' "$archive_path"
printf '%s\n' "$checksum_path"
