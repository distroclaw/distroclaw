#!/usr/bin/env bash
# Checks a DistroClaw DMG on this Mac, the counterpart of scripts/windows-smoke-test.ps1 and
# scripts/windows-model-test.ps1: mounts the DMG read-only, copies the app into a temporary folder,
# runs its self-test (database, documents, engine), then with the bundled engine downloads a model
# from the DistroClaw library, chats with it once and deletes it again, making the same engine calls
# as the app. Nothing is installed; the temporary folder, model included, is deleted at the end.
#
#   bash scripts/macos-smoke-test.sh <DistroClaw_macOS_arm64.dmg> [model, default qwen3.5:0.8b]

set -euo pipefail
dmg=${1:?usage: bash scripts/macos-smoke-test.sh <dmg> [model]}
model=${2:-qwen3.5:0.8b}
library=${DISTROCLAW_MODEL_LIBRARY:-models.distroclaw.com}
work=$(mktemp -d "${TMPDIR:-/tmp}/distroclaw-smoke.XXXXXX")
engine_pid=""

cleanup() {
  [ -n "$engine_pid" ] && kill "$engine_pid" 2>/dev/null && wait "$engine_pid" 2>/dev/null || true
  hdiutil detach -quiet "$work/mnt" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

passed() { echo "- passed: $*"; }
failed() { echo "- FAILED: $*" >&2; exit 1; }
json() { python3 -I -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

echo "# macOS smoke test"
echo "DMG: $(du -h "$dmg" | cut -f1), SHA-256 $(shasum -a 256 "$dmg" | cut -d' ' -f1)"
echo "macOS $(sw_vers -productVersion) on $(uname -m)"

# ---------- the app from the DMG ----------
mkdir "$work/mnt"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$work/mnt" "$dmg"
[ -d "$work/mnt/DistroClaw.app" ] && passed "DMG holds DistroClaw.app" || failed "no DistroClaw.app in the DMG"
[ -L "$work/mnt/Applications" ] && passed "DMG has the Applications shortcut" || failed "no Applications shortcut in the DMG"
ditto "$work/mnt/DistroClaw.app" "$work/DistroClaw.app"
hdiutil detach -quiet "$work/mnt"
app="$work/DistroClaw.app"
exe="$app/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$app/Contents/Info.plist")"
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
echo "Version $version"
codesign --verify --deep --strict "$app" && passed "code signature of the bundle is intact" || failed "code signature broken"
[ -x "$app/Contents/Resources/engine/ollama" ] && passed "AI engine bundled" || failed "no AI engine in the app"

# ---------- self-test ----------
"$exe" --self-test "$work/self-test.json" >/dev/null || failed "self-test exited with $?"
for check in database documents engine; do
  result=$(json "next(c['result'] + ': ' + c['detail'] for c in d['checks'] if c['name'] == '$check')" < "$work/self-test.json")
  [[ $result == passed:* ]] && passed "self-test $check ($result)" || failed "self-test $check ($result)"
done

# ---------- download → chat → delete, as the app does ----------
port=$(python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
base="http://127.0.0.1:$port"
mkdir "$work/models" "$work/home"
env -i PATH=/usr/bin:/bin HOME="$work/home" OLLAMA_HOST="127.0.0.1:$port" OLLAMA_MODELS="$work/models" \
  OLLAMA_KEEP_ALIVE=30m OLLAMA_NOPRUNE=1 OLLAMA_NO_CLOUD=1 OLLAMA_NUM_PARALLEL=1 OLLAMA_MAX_LOADED_MODELS=2 OLLAMA_DEBUG=0 \
  "$app/Contents/Resources/engine/ollama" serve > "$work/engine.log" 2>&1 &
engine_pid=$!
for _ in $(seq 120); do curl -sf "$base/api/version" >/dev/null && break; sleep 0.5; done
curl -sf "$base/api/version" >/dev/null && passed "engine starts" || failed "engine did not start: $(tail -3 "$work/engine.log")"

source="$library/distroclaw/$model"
start=$(date +%s)
status=$(curl -sf -X POST "$base/api/pull" -d "{\"model\":\"$source\",\"stream\":false}" | json "d.get('status')")
[ "$status" = success ] && passed "download $source in $(( $(date +%s) - start )) s" || failed "download: $status"
curl -sf -X POST "$base/api/copy" -d "{\"source\":\"$source\",\"destination\":\"$model\"}" >/dev/null
curl -sf -X DELETE "$base/api/delete" -d "{\"model\":\"$source\"}" >/dev/null
names=$(curl -sf "$base/api/tags" | json "' '.join(m['name'] for m in d['models'])")
[ "$names" = "$model" ] && passed "installed under its catalogue name only ($names)" || failed "model list: $names"

curl -sf -X POST "$base/api/chat" -d "{\"model\":\"$model\",\"stream\":false,\"think\":false,\"options\":{\"num_ctx\":4096},\"messages\":[{\"role\":\"user\",\"content\":\"What is 2 + 3? Answer with one short sentence.\"}]}" > "$work/chat.json"
answer=$(json "d['message']['content'].strip()[:80]" < "$work/chat.json")
[ -n "$answer" ] && passed "chat answers ($answer)" || failed "empty answer"
echo "- note: speed: $(json "'prompt %.0f tokens/s, answer %.0f tokens/s' % (d['prompt_eval_count']/(d['prompt_eval_duration']/1e9), d['eval_count']/(d['eval_duration']/1e9))" < "$work/chat.json")"

curl -sf -X DELETE "$base/api/delete" -d "{\"model\":\"$model\"}" >/dev/null
names=$(curl -sf "$base/api/tags" | json "' '.join(m['name'] for m in d['models'])")
[ -z "$names" ] && passed "deleted from the model list" || failed "still listed: $names"
left=$(find "$work/models/blobs" -type f -size +1M 2>/dev/null | wc -l | tr -d ' ')
[ "$left" = 0 ] && passed "its files are deleted from disk" || failed "$left model files left on disk"

echo
echo "All checks passed."
