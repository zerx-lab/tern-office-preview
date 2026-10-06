#!/bin/sh
# Builds the native renderer and installs it into the plugin's data directory:
#   <data>/bin/office-preview           the binary the plugin runs
#   <data>/bin/office-preview.version   the crate version it was built from
#
# Usage: scripts/build.sh [DATA_DIR]
# DATA_DIR defaults to $TERN_PLUGIN_DATA (set when the plugin runs this script), else Tern's
# plugin-data folder for this platform. Build output goes to <data>/target, never into the
# plugin folder: Tern reloads a plugin whenever its folder changes.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
id=office-preview

data=${1:-${TERN_PLUGIN_DATA:-}}
if [ -z "$data" ]; then
	if [ -n "${TERN_CONFIG_DIR:-}" ]; then
		data="$TERN_CONFIG_DIR/plugin-data/$id"
	elif [ "$(uname -s)" = Darwin ]; then
		data="$HOME/Library/Application Support/Tern/plugin-data/$id"
	else
		data="${XDG_STATE_HOME:-$HOME/.local/state}/tern/plugin-data/$id"
	fi
fi

# Tern's daemon may not have the login shell's PATH.
PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH
if ! command -v cargo >/dev/null 2>&1; then
	echo "未找到 cargo：请先安装 Rust 工具链（https://rustup.rs），再按 b 重新构建" >&2
	exit 127
fi

export CARGO_TARGET_DIR="$data/target"
cargo build --release --quiet --manifest-path "$here/native/Cargo.toml"

mkdir -p "$data/bin"
cp "$CARGO_TARGET_DIR/release/office-preview" "$data/bin/office-preview.tmp"
mv -f "$data/bin/office-preview.tmp" "$data/bin/office-preview"
sed -n 's/^version = "\(.*\)"$/\1/p' "$here/native/Cargo.toml" | head -n 1 >"$data/bin/office-preview.version"
echo "$data/bin/office-preview"
