sway-scratchpad (Rust)
======================

A Rust rewrite of [emilyyyylime/sway-scratchpad](https://github.com/emilyyyylime/sway-scratchpad):
launch an app as a sized, positioned floating window on Sway or i3, and toggle it with the same
command. It speaks IPC directly instead of shelling out to `swaymsg`, `jq`, `sed` and `md5sum`.

It is a drop-in replacement: same flags and same window marks, so existing `bindsym` lines and
already-open windows keep working.

Usage
-----
```
sway-scratchpad [-a <anchor>] [-d <size>] [-p <pos>] [-s <screen>] [-m <edge>] [-w <wm>] [-t] [-v] [-V] <command>

 -a <anchor>   Anchor point: {top,centre,bottom}-{left,centre,right}, or short: tl, tc, br, ...
               Default centre-centre.
 -d <size>     WIDTHxHEIGHT in pixels or percent of the screen. Default 50%x50%.
 -m <edge>     Slide in from (and out to) an edge: top, left, bottom, right / t, l, b, r.
 -p <pos>      X,Y offset from the anchor in pixels; may be negative. Default 0,0.
 -s <screen>   Output name. Defaults to the focused (sway) or primary (i3) output.
 -t            Toggle: hide the window if it is on the focused workspace.
 -v            Verbose.
 -V            Print version.
 -w <wm>       sway or i3.
 <command>     Command that opens the window, run if it isn't open yet.

 --timeout <secs>   How long to wait for the window to appear. Default 5.
 --app-id <id>      Only accept a window with this app_id (Wayland).
 --class <class>    Only accept a window with this X11 class.
```

Options bundle like `getopts` (`-tmt -atc`) and parsing stops at the first non-option, so
`sway-scratchpad -t kitty -o opacity=0.7` works. The command's arguments are joined with spaces and
run with `sh -c`.

Examples
--------
Dropdown terminal:
```
bindsym $mod+Ctrl+Return exec --no-startup-id sway-scratchpad -tmt -atc -d 60%x60% -- kitty -o background_opacity=0.7
```

Browser app in the top-right corner:
```shell
sway-scratchpad -d900x500 -mr -tatr -- brave --app=https://music.apple.com/en/browse
```

Differences from the Bash version
---------------------------------
- **The right window gets captured.** The script could mark whatever window was focused instead of
  the one it launched, and from then on every toggle moved that window. This version picks the new
  window whose process descends from the command.
- **Single-instance apps** (`brave --app=…`, `kitty --single-instance`) open their window from an
  already-running process. If the command exits successfully, a window that opens within 2 s is
  taken. Use `--app-id` or `--class` when that guess isn't good enough.
- **No duplicates or strays:** a double keypress doesn't launch twice, and if the window doesn't
  appear before the timeout, the launched processes are killed.
- **Multi-monitor:** `-s` places the window on that screen, and slide animations start at its edge.

Installation
------------
```shell
cargo install --path .
```
or, on Arch: `cd packaging/arch && makepkg -si`.

Testing
-------
```shell
cargo test             # unit tests
tests/integration.sh   # end-to-end in a throwaway headless sway; needs sway, jq, foot
```

License
-------
GPL-3.0-or-later, see [LICENSE](LICENSE). The original script is public domain (Unlicense).
