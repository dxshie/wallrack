#!/usr/bin/env bash
# Rofi script-mode wrapper around `wallrack`. All indexing, filtering,
# favorites, and state management live in the Rust binary; this script
# is just rofi protocol plumbing.
#
# hard requirements:
# - hyprland
# - rofi
# - awww (wallpaper mode)
# - wallrack
# soft requirements:
# - linux-wallpaperengine (WE mode)
# - mako, matugen, pywalfox
#
# rofi script-mode keybindings:
#   Alt+1  kb-custom-1  toggle favorite on the highlighted entry
#   Alt+2  kb-custom-2  switch between all wallpapers and favorites view
#   Alt+3  kb-custom-3  rebuild the index (current integration)
#   Alt+4  kb-custom-4  switch between wallpaper and Wallpaper Engine mode
#   Alt+5  kb-custom-5  open tag filter selection

set -o pipefail

NOTIFY_OPTIONS=(-i "$DOTFILES/logos/we.png" "Wallrack")

# rofi appends the highlighted entry on every re-invocation; persist the
# initial rating arg so subsequent calls can find it.
if [[ -z "$ROFI_RETV" || "$ROFI_RETV" == "0" ]]; then
  filter_rating="${1:-All}"
  wallrack state set rating "$filter_rating" >/dev/null
fi
selection="${*: -1}"

picker_mode=$(wallrack state get picker_mode 2>/dev/null || echo wallpaper)
tag_mode=$(wallrack state get tag_mode 2>/dev/null || echo "")
drill_path=$(wallrack state get drill_path 2>/dev/null || echo "")

# ─── helpers ────────────────────────────────────────────────────────────────

extract_image_from_entry() {
  echo "${1##* - }"
}

# Background re-index: kicks the daemon if running, else runs `wallrack index`.
start_refresh_background() {
  setsid wallrack index --integration="$picker_mode" >/dev/null 2>&1 < /dev/null &
  disown 2>/dev/null || true
}

ensure_index() {
  local integration="$1"
  if ! wallrack state get _index_built_$integration >/dev/null 2>&1; then
    if [[ ! -f "$HOME/.cache/wallrack/$integration/index.json" ]]; then
      notify-send "${NOTIFY_OPTIONS[@]}" "Building $integration index for the first time — please wait..."
      wallrack index --integration="$integration"
      wallrack state set _index_built_$integration 1 >/dev/null
    fi
  fi
}

# ─── apply wrappers (with theming) ──────────────────────────────────────────

apply_wallpaper() {
  local image="$1" monitor="$2"
  if [[ -z "$monitor" ]]; then
    notify-send -u critical "${NOTIFY_OPTIONS[@]}" "No monitor selected."
    exit 1
  fi
  if command -v matugen &>/dev/null; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Matugen detected, setting theme."
    matugen image "$image" --source-color-index 0 --lightness-dark 0.0 --lightness-light 0.0 -t scheme-content
  fi
  if command -v makoctl &>/dev/null; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Mako detected, updating."
    "$XDG_CONFIG_HOME/mako/update-theme.sh"
  fi
  wallrack apply --integration=wallpaper --monitor="$monitor" "$image"
  exit 0
}

apply_we() {
  local folder="$1" monitor="$2"
  if [[ -z "$monitor" ]]; then
    notify-send -u critical "${NOTIFY_OPTIONS[@]}" "No monitor selected."
    exit 1
  fi
  wallrack apply --integration=we --monitor="$monitor" "$folder"
  # Optionally re-theme from the preview image.
  if command -v matugen &>/dev/null; then
    preview=$(jq -r '.preview // ""' "$folder/project.json" 2>/dev/null)
    if [[ -n "$preview" && -f "$folder/$preview" ]]; then
      notify-send "${NOTIFY_OPTIONS[@]}" "Matugen detected, setting theme."
      matugen image "$folder/$preview" --source-color-index 0 --lightness-dark 0.0 --lightness-light 0.0 -t scheme-content
    fi
  fi
  exit 0
}

# ─── ROFI callbacks ─────────────────────────────────────────────────────────

case "$ROFI_RETV" in
  1)
    # Item selected — route by ROFI_INFO payload.
    if [[ "$tag_mode" == "selecting" ]]; then
      if [[ "$ROFI_INFO" == tag:* ]]; then
        wallrack state set tag_filter "${ROFI_INFO#tag:}" >/dev/null
      fi
      wallrack state unset tag_mode >/dev/null
      wallrack view
      exit 0
    fi
    if [[ "$ROFI_INFO" == "back:" ]]; then
      wallrack state unset drill_path >/dev/null
      wallrack view
    elif [[ "$ROFI_INFO" == folder:* ]]; then
      wallrack state set drill_path "${ROFI_INFO#folder:}" >/dev/null
      wallrack state set view_mode all >/dev/null
      wallrack view
    elif [[ "$ROFI_INFO" == image:* ]]; then
      # Drill-down image selected — route to monitor picker.
      target="${ROFI_INFO#image:}"
      wallrack monitors --integration=wallpaper --target="$target"
    elif [[ -n "$ROFI_INFO" ]]; then
      # Monitor selected; ROFI_INFO carries the target. Apply detached so rofi closes.
      if [[ "$picker_mode" == "we" ]]; then
        ( apply_we "$ROFI_INFO" "$selection" ) >/dev/null 2>&1 < /dev/null &
      else
        ( apply_wallpaper "$ROFI_INFO" "$selection" ) >/dev/null 2>&1 < /dev/null &
      fi
      disown 2>/dev/null || true
    else
      # Top-level entry without info field shouldn't happen now (all entries
      # carry their id as info), but fall through to monitor picker just in case.
      target=$(extract_image_from_entry "$selection")
      wallrack monitors --integration="$picker_mode" --target="$target"
    fi
    exit 0
    ;;
  10)
    # kb-custom-1: toggle favorite on highlighted entry
    if [[ "$ROFI_INFO" == folder:* ]]; then
      # Folder rows aggregate many images and don't carry an entry id; refuse
      # to toggle so we don't end up adding the subfolder string as a bogus
      # favorite.
      notify-send "${NOTIFY_OPTIONS[@]}" "Folders aren't favoritable — drill in first."
      wallrack view
      exit 0
    fi
    if [[ "$ROFI_INFO" == image:* ]]; then
      target="${ROFI_INFO#image:}"
    else
      target=$(extract_image_from_entry "$selection")
    fi
    if [[ -n "$target" ]]; then
      result=$(wallrack favorites toggle --integration="$picker_mode" "$target")
      label="${target##*/}"
      notify-send "${NOTIFY_OPTIONS[@]}" "$result favorite: $label"
    fi
    wallrack view
    exit 0
    ;;
  11)
    # kb-custom-2: toggle all/favorites view; exits drill-down if active
    wallrack state unset drill_path >/dev/null
    current=$(wallrack state get view_mode 2>/dev/null || echo all)
    if [[ "$current" == "favorites" ]]; then
      wallrack state set view_mode all >/dev/null
    else
      fav_count=$(wallrack favorites list --integration="$picker_mode" --format=json | jq 'length' 2>/dev/null || echo 0)
      if [[ "$fav_count" == "0" ]]; then
        notify-send "${NOTIFY_OPTIONS[@]}" "No favorites yet — press Alt+1 on a wallpaper to add one."
        wallrack view
        exit 0
      fi
      wallrack state set view_mode favorites >/dev/null
    fi
    wallrack view
    exit 0
    ;;
  12)
    # kb-custom-3: rebuild index for current integration
    wallrack state unset drill_path >/dev/null
    start_refresh_background
    notify-send "${NOTIFY_OPTIONS[@]}" "Refreshing $picker_mode index in the background. Re-open the picker once done."
    wallrack view
    exit 0
    ;;
  13)
    # kb-custom-4: toggle between wallpaper and WE mode
    if [[ "$picker_mode" == "we" ]]; then
      wallrack state set picker_mode wallpaper >/dev/null
    else
      wallrack state set picker_mode we >/dev/null
    fi
    wallrack state set view_mode all >/dev/null
    wallrack state unset drill_path >/dev/null
    wallrack state unset tag_filter >/dev/null
    wallrack state unset tag_mode >/dev/null
    wallrack view
    exit 0
    ;;
  14)
    # kb-custom-5: toggle tag filter selection
    if [[ "$tag_mode" == "selecting" ]]; then
      wallrack state unset tag_mode >/dev/null
    else
      wallrack state set tag_mode selecting >/dev/null
    fi
    wallrack view
    exit 0
    ;;
esac

# Initial invocation: clear transient state, ensure index exists, render.
wallrack state reset-transient >/dev/null
ensure_index "$picker_mode" || exit 1
wallrack view
