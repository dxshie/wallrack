# wallrack

A picker-agnostic wallpaper manager. Indexes plain image directories and Steam
Workshop content (image wallpapers + Wallpaper Engine projects), generates
thumbnails, tracks favorites/tags/ratings (including user tag overrides),
persists picker state, applies wallpapers to specific monitors, and optionally
watches sources for changes via a daemon.

Output is structured (JSON or rofi script-mode) so any frontend — rofi, fuzzel,
walker, wofi, a TUI, a desktop widget — can drive it. A reference rofi
frontend ships in this repo, but is not required.

## Integrations

Three independent integrations, each with its own index, favorites bucket,
tag overrides, and per-integration backend commands:

| Key         | What it indexes                                          | Drilling | Applies via                |
| ----------- | -------------------------------------------------------- | -------- | -------------------------- |
| `wallpaper` | Plain images in `wallpaper.dirs`                         | yes      | image backend (e.g. awww)  |
| `we_image`  | Images extracted from Wallpaper Engine workshop projects | yes      | image backend (e.g. awww)  |
| `we`        | Wallpaper Engine projects, live                          | no       | `linux-wallpaperengine`    |

## Demo

[![preview](https://raw.githubusercontent.com/dxshie/wallrack/refs/heads/master/preview.jpg)](https://github.com/dxshie/wallrack)

## What wallrack does

- **Index** — scans configured directories (plain image dirs and/or a Steam
  Workshop dir) and produces a JSON index of entries with title, tags, rating,
  workshop id, subfolder, and a generated thumbnail path.
- **Filter / view** — favorites-only, by tag, by rating, drill into a
  workshop subfolder, group images by subfolder.
- **State** — persists picker mode, view mode, drill path, tag filter etc.
  so successive frontend invocations stay coherent.
- **Apply** — sets a wallpaper on a specific monitor via the configured
  backend command (default: `awww` for images, `linux-wallpaperengine` for WE
  projects). Runs `[hooks].pre_apply_hook` before and
  `[hooks].post_apply_hook` after if configured.
- **Daemon** (optional) — watches source directories and re-indexes
  incrementally when content changes.

## Requirements

The compositor/wallpaper daemon side is fully configurable per integration
(see [Per-integration backend commands](#per-integration-backend-commands)).
The list below covers the built-in defaults — substitute your own commands in
config to use a different stack.

**Hard**

- A wallpaper backend for at least one integration you intend to use:
  - `wallpaper` / `we_image` → an image-setting daemon (default: `awww`)
  - `we` → [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine)
- A way to enumerate monitors (default: `hyprctl` from Hyprland; override
  with any command that prints monitor names line-by-line — `swaymsg`,
  `xrandr`, `wlr-randr`, …).

**Soft**

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
  inputs.wallrack.url = "github:dxshie/wallrack";
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

First run writes a default config to `~/.config/wallrack/config.toml`. The
minimal sane config (the maintainer's reference setup, hyprland + awww, is
baked in as defaults — you only set backend keys to override):

```toml
[hooks]
# pre_apply_hook  = ""
# post_apply_hook = "matugen image \"$WALLRACK_WALLPAPER\""

[thumbnails]
size = 256

[wallpaper]
dirs = ["~/Pictures"]

[wallpaper_engine_image]
# workshop_dir = "~/.local/share/Steam/steamapps/workshop/content/431960"
# falls back to wallpaper_engine.workshop_dir if unset

[wallpaper_engine]
workshop_dir = "~/.local/share/Steam/steamapps/workshop/content/431960"
```

### Per-integration backend commands

Each integration has a `[<name>.backend]` section with three optional keys —
unset values fall back to the hyprland + awww defaults shipped with the
binary. Templates use `{{image}}`, `{{monitor}}`, `{{folder}}` and
`{{workshop_id}}` (substituted as plain text and passed to `sh -c`, so quote
yourself).

```toml
# Image-based integrations (wallpaper, we_image) — these are the built-in defaults:
[wallpaper.backend]
apply_cmd         = 'awww img "{{image}}" --transition-type center -o "{{monitor}}"'
monitors_cmd      = "hyprctl monitors | awk '/^Monitor / {print $2}'"
current_image_cmd = "awww query | sed -nE 's/^[ :]*([^:]+):.*image: (.+)$/\\1\\t\\2/p'"

[wallpaper_engine_image.backend]
apply_cmd         = 'awww img "{{image}}" --transition-type center -o "{{monitor}}"'
monitors_cmd      = "hyprctl monitors | awk '/^Monitor / {print $2}'"
current_image_cmd = "awww query | sed -nE 's/^[ :]*([^:]+):.*image: (.+)$/\\1\\t\\2/p'"

# Live WE projects — launched detached via setsid; any running
# linux-wallpaperengine is killed and waited for before this fires.
[wallpaper_engine.backend]
apply_cmd    = 'uwsm app -- linux-wallpaperengine --screenshot-delay 1000 --screenshot ~/.cache/lock.jpg --autoplay-policy=no-user-gesture-required --no-audio-processing --disable-parallax --silent --no-fullscreen-pause --scaling fill --screen-root "{{monitor}}" --bg "{{workshop_id}}"'
monitors_cmd = "hyprctl monitors | awk '/^Monitor / {print $2}'"
```

Sample command sets for other stacks:

```toml
# hyprland + awww (default)
apply_cmd    = 'awww img "{{image}}" --transition-type center -o "{{monitor}}"'
monitors_cmd = "hyprctl monitors | awk '/^Monitor / {print $2}'"

# sway + swaybg (one bg process per output; users typically run a launcher
# script that maps the monitor → process). Adapt to your setup.
apply_cmd    = 'pkill -f "swaybg.*{{monitor}}"; swaybg -o "{{monitor}}" -i "{{image}}" -m fill &'
monitors_cmd = "swaymsg -t get_outputs -r | jq -r '.[].name'"

# X11 + feh
apply_cmd    = 'feh --bg-fill "{{image}}"'
monitors_cmd = "xrandr --listactivemonitors | awk 'NR>1 {print $4}'"
```

`current_image_cmd` is purely cosmetic — it powers the thumbnail next to each
monitor in the monitor picker. Leave it unset to skip that feature.

### Tags

Native tags come from `project.json` for WE entries; plain wallpapers have
none. wallrack maintains two related pieces of state:

- **Per-entry overrides** (`~/.cache/wallrack/tags.json`) — added/removed
  tag deltas layered on top of native tags every time the index is read.
- **Catalog** (`~/.cache/wallrack/tag_catalog.json`) — the set of "tags
  available to apply" per integration. Populated from native tags every time
  you index, augmented by anything you `tag add` / `tag set` / `tag create`,
  and used by the rofi picker to suggest existing tags in the add-tag prompt.

Per-entry operations:

```sh
wallrack tag add    --integration=wallpaper --id=/path/to/img.jpg cyberpunk
wallrack tag remove --integration=wallpaper --id=/path/to/img.jpg cyberpunk
wallrack tag set    --integration=we_image  --id=/path/to/img.jpg --tag=neon --tag=night
wallrack tag clear  --integration=wallpaper --id=/path/to/img.jpg
wallrack tag show   --integration=wallpaper --id=/path/to/img.jpg
```

Catalog operations:

```sh
wallrack tag available --integration=wallpaper --format=json
wallrack tag create    --integration=wallpaper cyberpunk            # declare without assigning
wallrack tag delete    --integration=wallpaper cyberpunk            # soft delete (catalog only)
wallrack tag delete    --integration=wallpaper --cascade cyberpunk  # also strip from every entry
```

`tag delete --cascade` writes a `removed` override for every entry that
currently has the tag (including native tags from `project.json`), which is
how native tags can be hidden — `tag clear` on those entries undoes it.

Per-entry overrides survive re-indexing: added tags are stored as
additive deltas, not as a frozen tag list, so a `project.json` update that
introduces new native tags still surfaces them.

### Ratings

Native ratings come from WE `project.json` (`contentrating` field: `Mature`,
`Questionable`, `Everyone`). The picker filters on the active rating, which
cycles via the Alt+6 keybinding through `All → Mature → Questionable →
Everyone → All` (`All` = no filter).

Per-entry rating overrides work the same way as tag overrides — stored in
`~/.cache/wallrack/rating_overrides.json` and applied on every read. Use
them to pin a rating on plain wallpapers (which have none natively) or to
correct a misclassified WE entry:

```sh
wallrack rating set   --integration=wallpaper --id=/path/to/img.jpg Mature
wallrack rating set   --integration=we_image  --id=/path/to/img.jpg All     # clear the rating
wallrack rating clear --integration=wallpaper --id=/path/to/img.jpg          # drop override → native rating returns
wallrack rating show  --integration=wallpaper --id=/path/to/img.jpg
```

`rating set <…> All` records an explicit "no rating" — the entry will pass
the `Everyone` / `Questionable` / `Mature` filters but match `All`. Use
`rating clear` to drop the override entirely so the native value (if any)
shines through again.

Cache and state live under `~/.cache/wallrack/` (per-integration index,
thumbnails, favorites, tag overrides, picker state).

## CLI

```sh
wallrack index --integration=wallpaper   # build/refresh the index + thumbs
wallrack index --integration=we_image    # the WE-image-scrape integration
wallrack index --integration=we          # live WE projects
wallrack list  --integration=we --format=json
wallrack view  --format=json             # render the current view per persisted state
wallrack tags  --integration=wallpaper --format=json
wallrack tag   add --integration=wallpaper --id=/path/to/image.jpg foo  # per-entry override
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

`--format=wofi`, `--format=walker`, and `--format=fuzzel` emit dmenu-ish
variants for the respective launchers.
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

## Reference frontends

[`wallrack_rofi_picker.sh`](./picker/wallrack_rofi_picker.sh) is a rofi script-mode wrapper that
drives wallrack — favorites, tag filter, drill-down, mode switch, monitor
picker, per-entry tag editing (Alt+5 spawns a nested rofi prompt to add or
remove tags). It's a complete working example, not the project's headline
feature. See the comment block at the top of the script for keybindings and
setup, and use it as a template if you want to build a frontend for a
different launcher.

[`wallrack_wofi_picker.sh`](./picker/wallrack_wofi_picker.sh) and
[`wallrack_fuzzel_picker.sh`](./picker/wallrack_fuzzel_picker.sh) mirror
the same feature set against wofi and fuzzel respectively. Both pickers
lack rofi's script-mode keybindings, so the action header rows
(`⊕ mode: …`, `⊕ view: …`, `⊕ tag: …`, `⊕ rating: …`, `⊕ refresh`) take
the place of Alt+1..6 and a sub-menu opens after picking an entry for the
apply / favorite / edit-tags actions.

Post-apply theming (matugen, mako scripts, etc.) is not handled by the
picker scripts. Configure `[hooks].post_apply_hook` (or
`[hooks].pre_apply_hook` for prep work) in `config.toml` instead — wallrack
runs those commands around every successful apply regardless of which
frontend triggered it.

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
