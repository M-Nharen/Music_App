# Qwertone

Play music on your laptop keyboard. Built for Arch + Hyprland (end4 rice), works over Bluetooth earbuds, and lets you remap every key.

> **Status: Part 1 of 9.** Low-latency engine, Piano layout, keybind editor, latency tools, keyboard test, end4 theme. See [PLAN.md](PLAN.md) for what's next.

## Install (Arch)

```bash
git clone https://github.com/M-Nharen/Music_App.git qwertone
cd qwertone
./scripts/setup-arch.sh        # packages, realtime group, PipeWire low-latency config
cargo run --release            # first build takes a few minutes
```

Or install it like a normal package (adds a launcher entry too):

```bash
cd packaging/arch
makepkg -si
qwertone
```

The piano sound comes from `soundfont-fluid` (the setup script installs it). Without it you get a simple built-in synth.

### Hyprland

Add to `~/.config/hypr/hyprland.conf` (or your end4 custom rules file):

```ini
# float it at a nice size, slightly see-through so end4's blur shows
windowrulev2 = float, class:^(qwertone)$
windowrulev2 = size 1180 820, class:^(qwertone)$
windowrulev2 = opacity 0.96 0.92, class:^(qwertone)$
```

Colours follow end4's Material You palette automatically (from `~/.local/state/quickshell/user/generated/colors.json`, or the older AGS `_material.scss`) and change when your wallpaper does.

## Playing

| Keys | What they do |
|---|---|
| `Z X C V B N M , . /` | White keys, lower octave (starts at C3) |
| `S D G H J L ;` | Black keys, lower octave |
| `Q W E R T Y U I O P [ ]` | White keys, upper octave (starts at C4) |
| `2 3 5 6 7 9 0 =` | Black keys, upper octave |
| `Space` | Sustain pedal (hold) |
| `↑ / ↓` | Octave up / down |
| `← / →` | Change key (transpose by a semitone) |
| `PgUp / PgDn` | Next / previous instrument |
| `Tab` | Soft / normal / loud |
| `Esc` | Stop all sound |

The big readout names whatever chord you're holding (try `Z C B` = C major, `N Q E` = A minor).

## Keybinds

Open the **Keybinds** tab:

- Click any key on the drawn keyboard to set what it does (a note, a control, or nothing).
- Or press **Rebind** next to an action and then press the new key. If that key was already in use, the two keys swap, so nothing loses its key without you noticing.
- **Save as new layout** makes a copy you can change freely. **Reset to Piano** restores the defaults.

Layouts are plain text files in `~/.config/qwertone/keymaps/`. Edit them in any editor; the app reloads them as soon as you save:

```toml
name = "My layout"
[bindings]
Z = "note 0"      # semitones above the lowest C (C3 at octave 0)
S = "note 1"
Space = "sustain"
Up = "octave+"
```

Actions: `note <n>`, `sustain`, `octave+`, `octave-`, `transpose+`, `transpose-`, `instrument+`, `instrument-`, `velocity`, `panic`.

Settings live in `~/.config/qwertone/config.toml` and reload on save too.

## Bluetooth earbuds (Galaxy Buds Live, Sony XM4…)

Bluetooth adds roughly 150–250 ms of delay. That comes from the earbuds and the Bluetooth codec, not from Qwertone. To get the most out of it:

1. **Audio & latency → Output.** If a lower-delay codec is available, click **Switch to a2dp-sink-sbc**. SBC has less delay than AAC or LDAC (LDAC on the XM4 has the most).
2. **Run the latency test** with each pair and press **Save**. Later parts use this number to line up songs and falling notes with what you actually hear.
3. **Choose an instrument per device.** Sounds marked ✦ (pads, strings, organ, choir) have soft attacks that hide the delay. Qwertone switches to it automatically when those earbuds connect.
4. Wired earphones (any cheap pair) bring the delay down to about 5 ms with no setup.

## Troubleshooting

- **Crackles or pops:** Audio & latency → Buffer size → 256.
- **No sound:** check the Audio engine card for an error, then click **Restart audio**. Make sure PipeWire is running: `systemctl --user status pipewire`.
- **Output card says it can't ask PipeWire:** install `libpulse` (it provides `pactl`).
- **Some keys don't light up when held together:** that's your laptop keyboard (ghosting). Check it in the **Keyboard test** tab.
- **Build fails on `pipewire`/`bindgen`:** install `clang`, or build without the native PipeWire backend: `cargo run --release --no-default-features`.

## Project layout

```
src/
  audio/        real-time engine (PipeWire via cpal), SoundFont + built-in synth
  keymap.rs     keybind model and TOML format
  config.rs     settings, keymap files, SoundFont search, file watching
  devices.rs    current output / Bluetooth codec detection via pactl
  latency.rs    tap-along latency measurement
  theme.rs      end4 / Material You colours and fonts
  music.rs      note names, chord naming, instruments
  ui/           egui app and custom widgets
```

Run the tests with `cargo test`.
