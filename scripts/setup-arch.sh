#!/usr/bin/env bash
# Qwertone setup for Arch Linux (works on end4 / Hyprland setups).
# - installs what's needed to build and run
# - adds your user to the `realtime` group so audio can run at real-time priority
# - lets PipeWire go down to very small buffers when an app asks for them
# Nothing here changes how other apps sound: PipeWire only uses small buffers
# while an app (like Qwertone) requests them.
set -euo pipefail

say() { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }

say "Installing packages (needs sudo)"
sudo pacman -S --needed \
    rust clang pkgconf alsa-lib \
    pipewire pipewire-pulse pipewire-alsa wireplumber libpulse \
    soundfont-fluid realtime-privileges

if ! id -nG "$USER" | grep -qw realtime; then
    say "Adding $USER to the realtime group (log out and back in afterwards)"
    sudo usermod -aG realtime "$USER"
fi

conf_dir="$HOME/.config/pipewire/pipewire.conf.d"
conf="$conf_dir/10-qwertone-low-latency.conf"
say "Writing $conf"
mkdir -p "$conf_dir"
cat > "$conf" <<'EOF'
# Added by Qwertone. Lets apps request buffers as small as 32 frames
# (0.7 ms at 48 kHz). The normal default for other apps is unchanged.
context.properties = {
    default.clock.rate          = 48000
    default.clock.allowed-rates = [ 44100 48000 ]
    default.clock.min-quantum   = 32
}
EOF

say "Restarting PipeWire"
systemctl --user restart pipewire pipewire-pulse wireplumber || true

say "Done. If you were added to the realtime group, log out and back in once."
echo "Then build and run:  cargo run --release"
