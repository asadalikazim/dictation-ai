#!/usr/bin/env bash
# Downloads the Whisper models that get bundled into the app.
#
#   scripts/fetch-models.sh
#
# Files land in src-tauri/resources/models/ (gitignored). Each download is
# verified against a pinned SHA-256, files already present and valid are
# skipped, and an interrupted download resumes. Run once before building.
set -euo pipefail

cd "$(dirname "$0")/.."
DEST="src-tauri/resources/models"
BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"

# file                          sha256
MODELS=(
  "ggml-base.en.bin             a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"
  "ggml-small.en.bin            c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d"
  "ggml-large-v3-turbo-q5_0.bin 394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
)

sha_ok() { [ "$(shasum -a 256 "$1" | cut -d' ' -f1)" = "$2" ]; }

mkdir -p "$DEST"
for entry in "${MODELS[@]}"; do
  read -r file sha <<<"$entry"
  target="$DEST/$file"

  if [ -f "$target" ] && sha_ok "$target" "$sha"; then
    echo "ok     $file (already present)"
    continue
  fi
  rm -f "$target"

  # A finished-but-unrenamed .part file makes curl's resume request fail.
  if [ ! -f "$target.part" ] || ! sha_ok "$target.part" "$sha"; then
    echo "fetch  $file"
    curl --fail --location --retry 3 --continue-at - --progress-bar \
      --output "$target.part" "$BASE_URL/$file" \
      || { rm -f "$target.part"; curl --fail --location --retry 3 --progress-bar \
             --output "$target.part" "$BASE_URL/$file"; }
  fi

  if ! sha_ok "$target.part" "$sha"; then
    echo "checksum mismatch for $file" >&2
    rm -f "$target.part"
    exit 1
  fi
  mv "$target.part" "$target"
  echo "done   $file"
done
echo "All models present in $DEST"
