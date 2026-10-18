#!/usr/bin/env fish
# Execute via xvfb-run; only the isolated virtual display is captured.
set -e WAYLAND_DISPLAY
set -gx XDG_SESSION_TYPE x11
set -gx LIBGL_ALWAYS_SOFTWARE 1
timeout 8s ./target/debug/rom-art-rs > /tmp/rom-art-gui.log 2>&1 &
set app_pid $last_pid
sleep 3
magick import -window root docs/interface-breeze-dark.png
set capture_status $status
wait $app_pid
if test $capture_status -ne 0
    cat /tmp/rom-art-gui.log
    exit $capture_status
end
