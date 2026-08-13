#!/usr/bin/env bash
set -euo pipefail

tag=${1:?release tag required}
app_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$app_version" ] || {
  echo "Cargo.toml has no workspace version" >&2
  exit 1
}
tag_version=${tag#v}
[ "$tag" != "$tag_version" ] && [ "$tag_version" = "$app_version" ] || {
  echo "release tag $tag disagrees with embedded app version $app_version" >&2
  exit 1
}
