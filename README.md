# wallrack

A picker-agnostic wallpaper manager. Indexes plain image directories and Steam
Workshop content (image wallpapers + Wallpaper Engine projects), generates
thumbnails, tracks favorites/tags/ratings, persists picker state, applies
wallpapers to specific monitors, and optionally watches sources for changes
via a daemon.

Output is structured (JSON or rofi script-mode) so any frontend — rofi, fuzzel,
walker, wofi, a TUI, a desktop widget — can drive it. A reference rofi
frontend ships in this repo, but is not required.

## Demo

<!-- TODO: replace with real video link -->
[![demo video](https://placehold.co/640x360?text=demo+video+coming+soon)](https://example.com/wallrack-demo)

## What wallrack does

- **Index** — scans configured directories (plain image dirs and/or a Steam
  Workshop dir) and produces a JSON index of entries with title, tags, rating,
  workshop id, subfolder, and a generated thumbnail path.
- **Filter / view** — favorites-only, by tag, by rating, drill into a
  workshop subfolder, group images by subfolder.
- **State** — persists picker mode, view mode, drill path, tag filter etc.
  so successive frontend invocations stay coherent.
- **Apply** — sets a wallpaper on a specific monitor (currently via `awww` /
  `swww` for images and `linux-wallpaperengine` for WE projects).
- **Daemon** (optional) — watches source directories and re-indexes
  incrementally when content changes.

## Requirements

**Hard**

- A wallpaper backend for at least one integration:
  - `wallpaper` integration → [awww](https://github.com/LGFae/swww) or `swww`
  - `we` integration → [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine)
- `hyprctl` — used to enumerate monitors when applying wallpapers.

**Soft**

- [matugen](https://github.com/InioX/matugen) — generate a theme from the
  selected wallpaper. Driven by the frontend, not wallrack itself.
- Any picker (rofi / fuzzel / walker / …) if you want a UI.

## Install

### Cargo

```sh
cargo build --release
cp target/release/wallrack ~/.local/bin/
```

A debug build is roughly 7× slower at thumbnail generation than release —
always install the release binary.

### Nix flake

```nix
# flake.nix
{
  inputs.wallrack.url = "github:youruser/wallrack";
  outputs = { self, nixpkgs, wallrack, ... }: {
    # NixOS
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      modules = [{
        environment.systemPackages = [ wallrack.packages.x86_64-linux.default ];
      }];
    };

    # home-manager
    homeConfigurations.you = ...; # add wallrack.packages.${system}.default to home.packages
  };
}
```

The flake exposes:

- `packages.<system>.wallrack` (and `.default`) — the Rust binary plus the
  bundled `wallrack-rofi-picker` reference frontend.
- `overlays.default` — adds `wallrack` to a nixpkgs overlay.
- `devShells.default` — `cargo`/`rustc`/`rust-analyzer`/`clippy`/`rustfmt`
  for hacking on the project itself (`nix develop` or via direnv `use flake`).

## Configuration

First run writes a default config to `~/.config/wallrack/config.toml`:

```toml
[thumbnails]
size = 256

[wallpaper]
dirs = []
steam_workshop_dir = "~/.local/share/Steam/steamapps/workshop/content/431960"

[wallpaper_engine]
workshop_dir = "~/.local/share/Steam/steamapps/workshop/content/431960"
```

- `wallpaper.dirs` — plain image directories, recursive.
- `wallpaper.steam_workshop_dir` — Workshop dir scanned for image-based
  wallpapers. Defaults to the WE workshop dir if unset.
- `wallpaper_engine.workshop_dir` — Workshop dir scanned for live WE projects.

Cache and state live under `~/.cache/wallrack/` (index, thumbnails, favorites,
picker state).

## CLI

```sh
wallrack index --integration=wallpaper   # build/refresh the index + thumbs
wallrack list  --integration=we --format=json
wallrack view  --format=json             # render the current view per persisted state
wallrack tags  --integration=wallpaper --format=json
wallrack favorites toggle --integration=wallpaper /path/to/image.jpg
wallrack monitors --integration=wallpaper --target=/path/to/image.jpg
wallrack apply    --integration=wallpaper --monitor=DP-1 /path/to/image.jpg
wallrack state  set view_mode favorites
wallrack info                            # show resolved paths & config
```

`--format=rofi` is also accepted on commands that emit entries. The protocol
is documented in `rofi-script(5)` — each row carries `icon` and `info`
metadata so a picker that supports script mode can render thumbnails and
route selections without parsing the display text.

`--format=json` is the format for any non-rofi frontend. Entries look like:

```json
{
  "integration": "wallpaper",
  "id": "/path/to/image.jpg",
  "title": "Some Workshop Title",
  "source": "/path/to/image.jpg",
  "thumb": "/home/u/.cache/wallrack/wallpaper/thumbs/<hash>_<name>.jpg",
  "tags": ["scenery", "night"],
  "rating": "Everyone",
  "workshop_id": "431960",
  "subfolder": "directories/customdirectory",
  "favorite": true,
  "info": null
}
```

## Daemon

```sh
wallrack daemon start          # detach
wallrack daemon start --foreground
wallrack daemon status
wallrack daemon stop
```

The daemon watches the configured directories and re-indexes when content
changes, so frontends never have to trigger a manual rebuild.

## Reference frontend (rofi)

[`wallrack_rofi_picker.sh`](./picker/wallrack_rofi_picker.sh) is a rofi script-mode wrapper that
drives wallrack — favorites, tag filter, drill-down, mode switch, monitor
picker, optional matugen + mako theming. It's a complete working example,
not the project's headline feature. See the comment block at the top of the
script for keybindings and setup, and use it as a template if you want to
build a frontend for a different launcher.

## Writing your own frontend

A frontend is just a process that:

1. Calls `wallrack view --format=json` (or `--format=rofi`) to get entries.
2. Lets the user pick one and calls `wallrack monitors --format=json
   --integration=<i> --target=<id>` to get the monitor list.
3. Lets the user pick a monitor and calls `wallrack apply --integration=<i>
   --monitor=<name> <id>`.

Filter/sort state can be stashed via `wallrack state set/get/unset` so
multiple invocations of the same frontend (rofi script-mode style) share
state across calls.
