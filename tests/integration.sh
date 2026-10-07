#!/usr/bin/env bash
# End-to-end test against a throwaway headless sway instance.
# Requires: sway, swaymsg, jq, foot. Usage: tests/integration.sh [path/to/sway-scratchpad]
set -euo pipefail

BIN=$(realpath "${1:-target/release/sway-scratchpad}")
T=$(mktemp -d)
SWAY_PID=
cleanup() {
	local status=$?
	if [[ $status -ne 0 && -f "$T/sway.log" ]]; then
		echo "--- nested sway log (tail) ---"; tail -30 "$T/sway.log"
	fi
	[[ -n "$SWAY_PID" ]] && kill "$SWAY_PID" 2>/dev/null || true
	rm -rf "$T"
}
trap cleanup EXIT

cat > "$T/config" <<EOF
output * resolution 1920x1080
# New "target*" windows don't take focus: the case where the Bash version
# marked the focused window instead of the new one.
no_focus [app_id="^target"]
exec env > $T/env
EOF

WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
	sway -c "$T/config" > "$T/sway.log" 2>&1 &
SWAY_PID=$!
for _ in $(seq 50); do [[ -s "$T/env" ]] && break; sleep 0.1; done
[[ -s "$T/env" ]] || { echo "nested sway did not start"; cat "$T/sway.log"; exit 1; }

# Point everything at the nested instance, never the live session.
SWAYSOCK=$(sed -n 's/^SWAYSOCK=//p' "$T/env")
export SWAYSOCK I3SOCK=$SWAYSOCK
export WAYLAND_DISPLAY=$(sed -n 's/^WAYLAND_DISPLAY=//p' "$T/env")
unset DISPLAY

pass=0 fail=0
check() { # check <description> <command...>
	if "${@:2}"; then echo "ok   - $1"; pass=$((pass + 1)); else echo "FAIL - $1"; fail=$((fail + 1)); fi
}
tree() { swaymsg -t get_tree; }
# Workspace name containing the window with the given app_id.
ws_of() {
	tree | jq -r --arg a "$1" '
		.. | select(.type? == "workspace") | . as $ws
		| [.. | objects | select(.app_id? == $a)] | select(length > 0) | $ws.name' | head -1
}
marks_of() { tree | jq -r --arg a "$1" '[.. | objects | select(.app_id? == $a) | .marks[]] | join(",")'; }
count_app() { tree | jq --arg a "$1" '[.. | objects | select(.app_id? == $a)] | length'; }
wait_app() { for _ in $(seq 50); do [[ $(count_app "$1") -gt 0 ]] && return; sleep 0.1; done; return 1; }

# A bystander window that has focus throughout.
foot --app-id=bystander &>/dev/null &
wait_app bystander
swaymsg -q '[app_id="bystander"] focus'

echo "# launch: the new window is marked even though it doesn't get focus"
"$BIN" -t -- foot --app-id=target1 2>/dev/null
check "bystander carries no mark" test -z "$(marks_of bystander)"
check "target1 carries the mark" test -n "$(marks_of target1)"
check "target1 shown on workspace 1" test "$(ws_of target1)" = 1

echo "# toggle hides, toggle shows"
swaymsg -q '[app_id="target1"] focus'
"$BIN" -t -- foot --app-id=target1 2>/dev/null
check "target1 hidden" test "$(ws_of target1)" = __i3_scratch
check "bystander still visible" test "$(ws_of bystander)" = 1
"$BIN" -t -- foot --app-id=target1 2>/dev/null
check "target1 shown again" test "$(ws_of target1)" = 1
check "no second target1 spawned" test "$(count_app target1)" = 1

echo "# geometry: 60%x60% anchored top-centre with animation"
"$BIN" -t -- foot --app-id=target1 # hide
"$BIN" -t -mt -atc -d 60%x60% -- foot --app-id=target1
# Outer geometry: sway reports the title bar separately in deco_rect.
rect=$(tree | jq -c '.. | objects | select(.app_id? == "target1")
	| [.rect.x, .rect.y - .deco_rect.height, .rect.width, .rect.height + .deco_rect.height]')
check "rect is [384,0,1152,648] (got $rect)" test "$rect" = "[384,0,1152,648]"
"$BIN" -t -mt -atc -d 60%x60% -- foot --app-id=target1
check "animated hide ends in scratchpad" test "$(ws_of target1)" = __i3_scratch

echo "# toggle from another workspace brings the window over"
"$BIN" -t -- foot --app-id=target1 # show on workspace 1
swaymsg -q workspace 2
"$BIN" -t -- foot --app-id=target1
check "target1 moved to workspace 2" test "$(ws_of target1)" = 2

echo "# hide animation starts from where the window actually is"
swaymsg -q '[app_id="target1"] move absolute position 100 px 500 px'
log=$("$BIN" -v -t -ml -- foot --app-id=target1 2>&1)
check "animation starts at the moved position" grep -q "from Rect { x: 100, y: 500" <<< "$log"
check "target1 hidden" test "$(ws_of target1)" = __i3_scratch
swaymsg -q workspace 1

echo "# --app-id: only the matching window is taken, not another child window"
"$BIN" --app-id=target4 -- 'foot --app-id=decoy4 & foot --app-id=target4' 2>/dev/null
check "target4 marked" test -n "$(marks_of target4)"
wait_app decoy4
check "decoy4 (also a child) unmarked" test -z "$(marks_of decoy4)"

echo "# concurrent invocations launch only one instance"
"$BIN" -- foot --app-id=target2 2>/dev/null & p1=$!
"$BIN" -- foot --app-id=target2 2>/dev/null & p2=$!
wait "$p1" "$p2"
check "exactly one target2" test "$(count_app target2)" = 1

echo "# handoff: launcher exits, window comes from a reparented process"
# Arguments are joined and run by `sh -c`, like the Bash version's `$*`.
"$BIN" -- 'setsid -f foot --app-id=target3 </dev/null >/dev/null 2>&1; exit 0'
check "target3 marked" test -n "$(marks_of target3)"
check "bystander still unmarked" test -z "$(marks_of bystander)"

echo "# handoff: a foreign window that opened before the launcher exited loses"
(sleep 0.3; foot --app-id=intruder2 &>/dev/null) &
"$BIN" -- 'sleep 1.5; setsid -f foot --app-id=target5 </dev/null >/dev/null 2>&1; exit 0'
check "target5 marked" test -n "$(marks_of target5)"
check "intruder2 unmarked" test -z "$(marks_of intruder2)"

echo "# a foreign window opened after the handoff grace period is not taken"
(sleep 3; foot --app-id=intruder &>/dev/null) &
check "launcher that opens nothing times out" bash -c "! '$BIN' --timeout 4.5 -- true 2>/dev/null"
wait_app intruder
check "intruder carries no mark" test -z "$(marks_of intruder)"

echo "# a command that fails is reported and marks nothing"
check "failing command exits non-zero" bash -c "! '$BIN' -- false 2>/dev/null"
check "bystander still unmarked" test -z "$(marks_of bystander)"

echo "# timeout without a window marks nothing and kills the launched processes"
times_out() { ! "$BIN" --timeout 0.5 -- 'sleep 7.32 & sleep 7.33' 2>/dev/null; }
check "timeout exits non-zero" times_out
sleep 0.3
check "whole process group killed" bash -c '! pgrep -fa "sleep 7.3[23]"'
check "bystander still unmarked" test -z "$(marks_of bystander)"

echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
