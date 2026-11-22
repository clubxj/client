#!/usr/bin/env bash
set -uo pipefail

f=$(jq -r '.tool_response.filePath // .tool_input.file_path // empty')
[[ -n "$f" && -f "$f" ]] || exit 0
root=$(cd "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root" || exit 0
out=""
if [[ $f == "$root"/sdk/luau/*.luau ]]; then
	cd "$root/sdk/luau" || exit 0
	rel=${f#"$root"/sdk/luau/}
	stylua --config-path tools/stylua.toml "$rel" >/dev/null 2>&1
	out=$(luau-analyze "$rel" 2>&1 | grep -E '^\S+\([0-9]+,[0-9]+\): ' | grep -v ': TypeError' || true)
	if [[ -n "$out" ]]; then
		printf 'Lint issues in %s:\n%s\n' "$f" "$out" >&2
		exit 2
	fi
	exit 0
fi
[[ $f == *.rs && $f != "$root"/target/* && $f != "$root"/archive/* ]] || exit 0
dir=$(dirname "$f")
while [[ $dir != "$root" && ! -f $dir/Cargo.toml ]]; do dir=$(dirname "$dir"); done
cd "$dir" || exit 0
edition=$(grep -m1 '^edition' Cargo.toml | sed 's/[^0-9]//g')
rustfmt --edition "${edition:-2024}" "$f" >/dev/null 2>&1
out=$(timeout 150 cargo clippy --quiet --message-format=short 2>&1 | grep -F "${f#"$dir"/}" || true)

if [[ -n "$out" ]]; then
	printf 'Lint issues in %s:\n%s\n' "$f" "$out" >&2
	exit 2
fi
exit 0
