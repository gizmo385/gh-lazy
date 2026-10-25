#!/usr/bin/env bash
# Creates or updates the rewrite's labels from docs/rust-rewrite/labels.json. Requires gh and jq.
set -euo pipefail

REPO="${1:-gizmo385/gh-lazy}"
LABELS_FILE="$(dirname "$0")/../docs/rust-rewrite/labels.json"

jq -c '.[]' "$LABELS_FILE" | while read -r label; do
    name=$(jq -r '.name' <<<"$label")
    color=$(jq -r '.color' <<<"$label")
    description=$(jq -r '.description' <<<"$label")
    gh label create "$name" --repo "$REPO" --color "$color" --description "$description" --force
done
