#!/bin/sh
# Packages the plugin with prebuilt renderers, so users need no Rust toolchain:
#
#   dist/office-preview/                     an installable plugin folder
#     bin/macos/office-preview               universal (arm64 + x86_64), ad-hoc signed
#     bin/linux/office-preview-x86_64        static (musl)
#     bin/linux/office-preview-aarch64       static (musl)
#     bin/windows/office-preview.exe         x86_64 (also runs on Windows on Arm)
#     bin/VERSION                            the crate version, checked by lib/native.luau
#   dist/office-preview-<version>.tar.gz     the same folder, archived (and .zip when zip exists)
#
# Usage: scripts/package.sh [--out DIR] [--targets "macos linux windows"] [--no-sources]
#   --out         where to write (default: <repo>/dist, outside the plugin folder)
#   --targets     platforms to build (default: all three; a platform whose toolchain is
#                 missing is skipped with a warning)
#   --no-sources  leave native/ and scripts/build.sh out: smaller, but a platform without a
#                 prebuilt binary can then not build one
#
# Needs: cargo (rustup), on macOS for the macos target (lipo, codesign); cargo-zigbuild + zig
# for linux and windows. Build output goes to $CARGO_TARGET_DIR (default: a temp dir), never
# into the plugin folder.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
out="$(dirname "$here")/dist"
targets="macos linux windows"
sources=1
while [ $# -gt 0 ]; do
	case "$1" in
	--out) out=$2; shift 2 ;;
	--targets) targets=$2; shift 2 ;;
	--no-sources) sources=0; shift ;;
	*) echo "unknown option: $1" >&2; exit 2 ;;
	esac
done

PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/office-preview-dist-target}"
manifest="$here/native/Cargo.toml"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$manifest" | head -n 1)
stage="$out/office-preview"

warn() { printf '跳过 %s：%s\n' "$1" "$2" >&2; }
have() { command -v "$1" >/dev/null 2>&1; }
# Adds rustup targets that aren't installed yet (no-op without rustup).
want_targets() {
	have rustup || return 0
	installed=$(rustup target list --installed)
	for t in "$@"; do
		echo "$installed" | grep -qx "$t" || rustup target add "$t" >/dev/null
	done
}
release() { echo "$CARGO_TARGET_DIR/$1/release/office-preview$2"; }

rm -rf "$stage"
mkdir -p "$stage/bin"
cp "$here/plugin.toml" "$here/host.luau" "$here/window.luau" "$here/office.css" "$here/chrome.css" \
	"$here/README.md" "$stage/"
cp -R "$here/lib" "$stage/lib"
if [ "$sources" = 1 ]; then
	mkdir -p "$stage/native" "$stage/scripts"
	cp "$manifest" "$here/native/Cargo.lock" "$stage/native/"
	cp -R "$here/native/src" "$stage/native/src"
	cp "$here/scripts/build.sh" "$stage/scripts/"
fi

built=0
for target in $targets; do
	case "$target" in
	macos)
		if [ "$(uname -s)" != Darwin ] || ! have lipo; then
			warn macos "需要在 macOS 上打包"
			continue
		fi
		want_targets aarch64-apple-darwin x86_64-apple-darwin
		for t in aarch64-apple-darwin x86_64-apple-darwin; do
			MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked --quiet --manifest-path "$manifest" --target "$t"
		done
		mkdir -p "$stage/bin/macos"
		lipo -create -output "$stage/bin/macos/office-preview" \
			"$(release aarch64-apple-darwin "")" "$(release x86_64-apple-darwin "")"
		codesign --force --sign - "$stage/bin/macos/office-preview" 2>/dev/null
		built=$((built + 1))
		;;
	linux)
		if ! have cargo-zigbuild || ! have zig; then
			warn linux "需要 cargo-zigbuild 和 zig（cargo install cargo-zigbuild; brew install zig）"
			continue
		fi
		want_targets x86_64-unknown-linux-musl aarch64-unknown-linux-musl
		mkdir -p "$stage/bin/linux"
		for arch in x86_64 aarch64; do
			cargo zigbuild --release --locked --quiet --manifest-path "$manifest" --target "$arch-unknown-linux-musl"
			cp "$(release "$arch-unknown-linux-musl" "")" "$stage/bin/linux/office-preview-$arch"
		done
		built=$((built + 1))
		;;
	windows)
		if ! have cargo-zigbuild || ! have zig; then
			warn windows "需要 cargo-zigbuild 和 zig"
			continue
		fi
		want_targets x86_64-pc-windows-gnu
		cargo zigbuild --release --locked --quiet --manifest-path "$manifest" --target x86_64-pc-windows-gnu
		mkdir -p "$stage/bin/windows"
		cp "$(release x86_64-pc-windows-gnu .exe)" "$stage/bin/windows/office-preview.exe"
		built=$((built + 1))
		;;
	*)
		warn "$target" "未知平台（可选 macos linux windows）"
		;;
	esac
done

if [ "$built" = 0 ]; then
	echo "没有构建出任何平台的渲染器" >&2
	exit 1
fi
printf '%s\n' "$version" >"$stage/bin/VERSION"
chmod 755 "$stage"/bin/*/office-preview* 2>/dev/null || true

archive="$out/office-preview-$version"
rm -f "$archive.tar.gz" "$archive.zip"
tar -czf "$archive.tar.gz" -C "$out" office-preview
if have zip; then
	(cd "$out" && zip -qry "$archive.zip" office-preview)
fi

echo "office-preview $version"
(cd "$stage" && find bin -type f | sort | while read -r f; do printf '  %-36s %s\n' "$f" "$(du -h "$f" | cut -f1)"; done)
echo "  $stage"
ls -1 "$archive".* | sed 's/^/  /'
