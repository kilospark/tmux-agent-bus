#!/bin/bash
# Usage: ./bump-version.sh [major|minor|patch]
# Default: patch

set -e

TYPE=${1:-patch}
DIR="$(cd "$(dirname "$0")" && pwd)"

# Read current version from rust/Cargo.toml
CURRENT=$(grep '^version' "$DIR/rust/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

case "$TYPE" in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
  *) echo "Usage: $0 [major|minor|patch]"; exit 1 ;;
esac

NEW="${MAJOR}.${MINOR}.${PATCH}"

# Update version files
sed -i '' "s/^version = \".*\"/version = \"$NEW\"/" "$DIR/rust/Cargo.toml"
sed -i '' "s/\"version\": \".*\"/\"version\": \"$NEW\"/" "$DIR/package.json"

echo "$CURRENT -> $NEW"
