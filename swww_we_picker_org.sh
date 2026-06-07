#!/usr/bin/env bash
# Pick a wallpaper via rofi script mode — supports both swww/awww image
# wallpapers and Wallpaper Engine (linux-wallpaperengine) wallpapers.
#
# hard requirements:
# - hyprland
# - rofi
# - awww (wallpaper mode)
# soft requirements:
# - linux-wallpaperengine (WE mode)
# - mako
# - matugen
# - pywalfox
#
# rofi script-mode keybindings:
#   Alt+1  kb-custom-1  toggle favorite on the highlighted wallpaper
#   Alt+2  kb-custom-2  switch between all wallpapers and favorites view
#   Alt+3  kb-custom-3  rebuild the index (wallpaper or WE mode)
#   Alt+4  kb-custom-4  switch between wallpaper and Wallpaper Engine mode
#   Alt+5  kb-custom-5  open tag filter selection (reads tags from project.json)

set -o pipefail

# ─── Paths ───────────────────────────────────────────────────────────────────

NOTIFY_OPTIONS=(-i "$DOTFILES/logos/we.png" "AWWW Picker")
CACHE="$HOME/.cache/rofi-wall-thumbs"
INDEX="$CACHE/index.tsv"
WE_CACHE="$HOME/.cache/rofi-wall-we"
WE_INDEX="$WE_CACHE/index.tsv"
FAVORITES_FILE="$HOME/.cache/rofi-wall-favorites.list"
WE_FAVORITES_FILE="$HOME/.cache/rofi-wall-we-favorites.list"
WE_MONITOR_STATE_FILE="$HOME/.cache/rofi-wall-we-monitor-state"
STATE_DIR="$HOME/.cache/rofi-wall"
STATE_FILE="$STATE_DIR/state"
CONFIG_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/rofi-wall/config.json"
REFRESH_SCRIPT="$(dirname "$(readlink -f "$0")")/swww_we_picker_refresh.sh"
WE_REFRESH_SCRIPT="$(dirname "$(readlink -f "$0")")/swww_we_picker_we_refresh.sh"

mkdir -p "$CACHE" "$WE_CACHE" "$STATE_DIR"
touch "$FAVORITES_FILE" "$WE_FAVORITES_FILE"

# ─── Config ──────────────────────────────────────────────────────────────────

_load_config() {
  if [[ ! -f "$CONFIG_FILE" ]]; then
    mkdir -p "${CONFIG_FILE%/*}"
    printf '{\n  "steam_workshop_dir": "~/.local/share/Steam/steamapps/workshop/content/431960",\n  "wallpaper_dirs": []\n}\n' \
      > "$CONFIG_FILE"
  fi
  local raw
  raw=$(jq -r '.steam_workshop_dir // ""' "$CONFIG_FILE" 2>/dev/null)
  steam_workshop_dir="${raw/#\~/$HOME}"
  [[ -z "$steam_workshop_dir" ]] && steam_workshop_dir="$HOME/.local/share/Steam/steamapps/workshop/content/431960"

  mapfile -t wallpaper_dirs < <(
    jq -r '.wallpaper_dirs[]?' "$CONFIG_FILE" 2>/dev/null | while IFS= read -r d; do
      printf '%s\n' "${d/#\~/$HOME}"
    done
  )
}
_load_config

# ─── State helpers ───────────────────────────────────────────────────────────

_get_state() {
  local key="$1" default="$2" val
  val=$(grep -m1 "^${key}=" "$STATE_FILE" 2>/dev/null | cut -d= -f2-)
  printf '%s' "${val:-$default}"
}

_set_state() {
  local key="$1" val="$2" tmp
  tmp=$(mktemp "$STATE_DIR/state.XXXXXX")
  grep -v "^${key}=" "$STATE_FILE" > "$tmp" 2>/dev/null || true
  printf '%s=%s\n' "$key" "$val" >> "$tmp"
  mv "$tmp" "$STATE_FILE"
}

get_picker_mode()  { _get_state "picker_mode" "wallpaper"; }
set_picker_mode()  { _set_state "picker_mode" "$1"; }
get_view_mode()    { _get_state "view_mode" "all"; }
set_view_mode()    { _set_state "view_mode" "$1"; }
get_drill_path()   { _get_state "drill_path" ""; }
set_drill_path()   { _set_state "drill_path" "$1"; }
get_tag_filter()   { _get_state "tag_filter" ""; }
set_tag_filter()   { _set_state "tag_filter" "$1"; }
get_tag_mode()     { _get_state "tag_mode" ""; }
set_tag_mode()     { _set_state "tag_mode" "$1"; }

# rofi appends the highlighted entry to argv on every re-invocation, so the
# rating arg is only valid on the initial call. Persist it and read it back.
if [[ -z "$ROFI_RETV" || "$ROFI_RETV" == "0" ]]; then
  filter_rating="$1"
  _set_state "rating" "$filter_rating"
else
  filter_rating=$(_get_state "rating" "")
fi
selection="${*: -1}"

[[ -z "$filter_rating" ]] && filter_rating="All"

# Cache state once per invocation to avoid repeated file reads in hot paths
# (render_view + each keybinding handler would otherwise re-fork).
PICKER_MODE=$(get_picker_mode)
VIEW_MODE=$(get_view_mode)
DRILL_PATH=$(get_drill_path)
TAG_FILTER=$(get_tag_filter)
TAG_MODE=$(get_tag_mode)

# ─── Favorites ───────────────────────────────────────────────────────────────

_is_fav() {
  [[ -s "$1" ]] && grep -Fxq -- "$2" "$1"
}

_add_fav() {
  _is_fav "$1" "$2" || printf '%s\n' "$2" >> "$1"
}

_remove_fav() {
  local tmp
  tmp=$(mktemp)
  grep -Fxv -- "$2" "$1" > "$tmp" 2>/dev/null || true
  mv "$tmp" "$1"
}

toggle_favorite() {
  local image="$1"
  if _is_fav "$FAVORITES_FILE" "$image"; then
    _remove_fav "$FAVORITES_FILE" "$image"
    notify-send "${NOTIFY_OPTIONS[@]}" "Removed favorite: ${image##*/}"
  else
    _add_fav "$FAVORITES_FILE" "$image"
    notify-send "${NOTIFY_OPTIONS[@]}" "Added favorite: ${image##*/}"
  fi
}

toggle_we_favorite() {
  local folder="$1"
  local label="${folder%/}"
  label="${label##*/}"
  if _is_fav "$WE_FAVORITES_FILE" "$folder"; then
    _remove_fav "$WE_FAVORITES_FILE" "$folder"
    notify-send "${NOTIFY_OPTIONS[@]}" "Removed WE favorite: $label"
  else
    _add_fav "$WE_FAVORITES_FILE" "$folder"
    notify-send "${NOTIFY_OPTIONS[@]}" "Added WE favorite: $label"
  fi
}

# ─── Entry extraction ────────────────────────────────────────────────────────

# Strip "title - " or "★ title - " prefix to recover the image/folder path.
extract_image_from_entry() {
  echo "${1##* - }"
}

# ─── Index management ────────────────────────────────────────────────────────

_start_refresh_bg() {
  local script="$1" label="$2"
  if [[ ! -x "$script" ]]; then
    notify-send -u "critical" "${NOTIFY_OPTIONS[@]}" "$label refresh script missing: $script"
    return 1
  fi
  setsid "$script" >/dev/null 2>&1 < /dev/null &
  disown 2>/dev/null || true
}

_ensure_index() {
  local index_file="$1" script="$2" label="$3"
  [[ -f "$index_file" ]] && return 0
  if [[ ! -x "$script" ]]; then
    notify-send -u "critical" "${NOTIFY_OPTIONS[@]}" "$label refresh script missing: $script"
    return 1
  fi
  notify-send "${NOTIFY_OPTIONS[@]}" "Building $label index for the first time — please wait..."
  "$script"
}

start_refresh_background()     { _start_refresh_bg "$REFRESH_SCRIPT" "wallpaper"; }
start_we_refresh_background()  { _start_refresh_bg "$WE_REFRESH_SCRIPT" "WE"; }
ensure_index()                 { _ensure_index "$INDEX" "$REFRESH_SCRIPT" "wallpaper"; }
ensure_we_index()              { _ensure_index "$WE_INDEX" "$WE_REFRESH_SCRIPT" "WE"; }

# ─── Filtering ───────────────────────────────────────────────────────────────

_filter_rating() {
  if [[ -n "$filter_rating" && "$filter_rating" != "All" ]]; then
    awk -F'\t' -v r="$filter_rating" '$2 == r' "$1"
  else
    cat "$1"
  fi
}

_filter_by_tag() {
  local tag_filter="$1" col="$2"
  if [[ -n "$tag_filter" ]]; then
    awk -F'\t' -v t="$tag_filter" -v c="$col" '{
      n = split($c, tags, "|")
      for (i = 1; i <= n; i++) if (tags[i] == t) { print; break }
    }'
  else
    cat
  fi
}

# ─── Wallpaper entries (swww) ────────────────────────────────────────────────

emit_entries() {
  local prefix="${1-}"
  while IFS=$'\t' read -r title rating workshopid image thumb subfolder; do
    [[ -z "$image" ]] && continue
    printf "%s%s - %s\0icon\x1f%s\n" "$prefix" "$title" "$image" "$thumb"
  done
}

# Like emit_entries but groups images that share a subfolder into a single
# folder entry. Images directly in the project root are emitted individually.
# Folder entries carry info=folder:{path} so the picker can drill into them.
# Favorited root-level images are prefixed with ★.
emit_grouped_entries() {
  awk -F'\t' -v favs="$FAVORITES_FILE" '
    BEGIN { while ((getline f < favs) > 0) fav[f] = 1 }
    {
      title = $1; workshopid = $3; image = $4; thumb = $5; subfolder = $6
      if (subfolder == "") {
        star = (image in fav) ? "★ " : ""
        printf "%s%s - %s\0icon\x1f%s\n", star, title, image, thumb
      } else {
        key = workshopid "\034" subfolder
        if (!(key in seen)) {
          seen[key] = 1
          folder_path = image
          sub(/\/[^\/]*$/, "/", folder_path)
          printf "%s - %s\0icon\x1f%s\x1finfo\x1ffolder:%s\n", title, subfolder, thumb, folder_path
        }
      }
    }
  '
}

get_wallpaper_info() {
  _filter_rating "$INDEX" | _filter_by_tag "$TAG_FILTER" 7 | emit_grouped_entries
}

get_favorite_wallpaper_info() {
  [[ -s "$FAVORITES_FILE" ]] || return 0
  awk -F'\t' -v favs="$FAVORITES_FILE" '
    BEGIN { while ((getline f < favs) > 0) fav[f] = 1 }
    ($4 in fav)
  ' "$INDEX" | _filter_by_tag "$TAG_FILTER" 7 | emit_entries "★ "
}

# Emit all images directly inside folder_path as drill-down entries.
# Each entry carries info=image:{full_image_path} so the picker can
# distinguish these from monitor-selection entries (which carry a bare path).
# Favorited images are prefixed with ★.
get_folder_wallpaper_info() {
  local folder_path="$1"
  awk -F'\t' -v path="$folder_path" -v r="$filter_rating" -v favs="$FAVORITES_FILE" -v tf="$TAG_FILTER" '
    BEGIN { while ((getline f < favs) > 0) fav[f] = 1 }
    {
      rating = $2; image = $4; thumb = $5; tags_col = $7
      img_dir = image; sub(/\/[^\/]*$/, "/", img_dir)
      if (img_dir == path && (r == "" || r == "All" || rating == r)) {
        if (tf != "") {
          n = split(tags_col, tags, "|")
          found = 0
          for (i = 1; i <= n; i++) if (tags[i] == tf) { found = 1; break }
          if (!found) next
        }
        basename_img = image; sub(/.*\//, "", basename_img)
        star = (image in fav) ? "★ " : ""
        printf "%s%s\0icon\x1f%s\x1finfo\x1fimage:%s\n", star, basename_img, thumb, image
      }
    }
  ' "$INDEX"
}

# ─── Wallpaper entries (WE) ──────────────────────────────────────────────────

# Favorited WE entries (keyed by folder path) are prefixed with ★.
emit_we_entries() {
  awk -F'\t' -v favs="$WE_FAVORITES_FILE" '
    BEGIN { while ((getline f < favs) > 0) fav[f] = 1 }
    {
      title = $1; folder = $4; preview = $5
      if (folder == "") next
      star = (folder in fav) ? "★ " : ""
      printf "%s%s - %s\0icon\x1f%s\n", star, title, folder, preview
    }
  '
}

get_we_wallpaper_info() {
  ensure_we_index || return 1
  _filter_rating "$WE_INDEX" | _filter_by_tag "$TAG_FILTER" 6 | emit_we_entries
}

get_we_favorite_wallpaper_info() {
  ensure_we_index || return 1
  [[ -s "$WE_FAVORITES_FILE" ]] || return 0
  awk -F'\t' -v favs="$WE_FAVORITES_FILE" '
    BEGIN { while ((getline f < favs) > 0) fav[f] = 1 }
    ($4 in fav)
  ' "$WE_INDEX" | _filter_by_tag "$TAG_FILTER" 6 | emit_we_entries
}

get_tag_entries() {
  local index_file="$1" tag_col="$2"
  printf 'All tags\0info\x1ftag:\n'
  awk -F'\t' -v col="$tag_col" '
    {
      n = split($col, tags, "|")
      for (i = 1; i <= n; i++) if (tags[i] != "") all[tags[i]] = 1
    }
    END { for (t in all) printf "%s\0info\x1ftag:%s\n", t, t }
  ' "$index_file" | sort
}

# ─── Header / render ─────────────────────────────────────────────────────────

emit_header() {
  local prompt
  if [[ -n "$DRILL_PATH" ]]; then
    prompt="${DRILL_PATH%/}"
    prompt="${prompt##*/}"
    [[ -n "$TAG_FILTER" ]] && prompt="$prompt [$TAG_FILTER]"
  elif [[ "$PICKER_MODE" == "we" ]]; then
    prompt="WE"
    [[ "$VIEW_MODE" == "favorites" ]] && prompt="$prompt Favorites"
    [[ -n "$TAG_FILTER" ]] && prompt="$prompt [$TAG_FILTER]"
  else
    prompt="Wallpapers"
    if [[ "$VIEW_MODE" == "favorites" ]]; then
      prompt="$prompt Favorites"
    elif [[ -n "$filter_rating" && "$filter_rating" != "All" ]]; then
      prompt="$prompt ($filter_rating)"
    fi
    [[ -n "$TAG_FILTER" ]] && prompt="$prompt [$TAG_FILTER]"
  fi
  printf '\0prompt\x1f%s\n' "$prompt"
  printf '\0use-hot-keys\x1ftrue\n'
  if [[ -n "$DRILL_PATH" ]]; then
    printf '\0message\x1fAlt+1 fav | Alt+5 tag: %s  |  select ← Back to return\n' "${TAG_FILTER:-All}"
  else
    printf '\0message\x1fAlt+1 fav | Alt+2 view: %s | Alt+3 refresh | Alt+4 mode: %s | Alt+5 tag: %s\n' \
      "$VIEW_MODE" "$PICKER_MODE" "${TAG_FILTER:-All}"
  fi
}

render_tag_view() {
  printf '\0prompt\x1fFilter by Tag\n'
  printf '\0use-hot-keys\x1ftrue\n'
  printf '\0message\x1fSelect a tag — current: %s | Alt+5 to cancel\n' "${TAG_FILTER:-All}"
  if [[ "$PICKER_MODE" == "we" ]]; then
    ensure_we_index && get_tag_entries "$WE_INDEX" 6
  else
    ensure_index && get_tag_entries "$INDEX" 7
  fi
}

render_view() {
  if [[ "$TAG_MODE" == "selecting" ]]; then
    render_tag_view
    return
  fi

  local entries_tmp
  entries_tmp=$(mktemp)

  if [[ -n "$DRILL_PATH" ]]; then
    printf '← Back\0info\x1fback:\n' > "$entries_tmp"
    get_folder_wallpaper_info "$DRILL_PATH" >> "$entries_tmp"
  elif [[ "$VIEW_MODE" == "favorites" ]]; then
    if [[ "$PICKER_MODE" == "we" ]]; then
      get_we_favorite_wallpaper_info > "$entries_tmp"
    else
      get_favorite_wallpaper_info > "$entries_tmp"
    fi
    if [[ ! -s "$entries_tmp" ]]; then
      set_view_mode "all"; VIEW_MODE="all"
      if [[ "$PICKER_MODE" == "we" ]]; then
        get_we_wallpaper_info > "$entries_tmp"
      else
        get_wallpaper_info > "$entries_tmp"
      fi
    fi
  elif [[ "$PICKER_MODE" == "we" ]]; then
    get_we_wallpaper_info > "$entries_tmp"
  else
    get_wallpaper_info > "$entries_tmp"
  fi

  emit_header
  cat "$entries_tmp"
  rm -f "$entries_tmp"
}

# ─── Dependencies ────────────────────────────────────────────────────────────

dependencies_check() {
  local cmd
  for cmd in rofi hyprctl awww jq; do
    if ! command -v "$cmd" &>/dev/null; then
      notify-send -u "critical" "${NOTIFY_OPTIONS[@]}" "$cmd is required."
      exit 1
    fi
  done
  if [[ ! -d "$steam_workshop_dir" ]]; then
    notify-send -u "critical" "${NOTIFY_OPTIONS[@]}" "Steam workshop dir not found: $steam_workshop_dir — check $CONFIG_FILE"
    exit 1
  fi
}

# ─── Monitor selection (swww) ────────────────────────────────────────────────

select_monitor_with_path() {
  local image="$1"

  mapfile -t awww_lines < <(awww query)
  declare -A monitor_images
  local monitors
  mapfile -t monitors < <(hyprctl monitors -j | jq -r '.[].name')

  local line monitor_name image_path
  for line in "${awww_lines[@]}"; do
    monitor_name=$(echo "$line" | grep -oP '^: \K[^:]+')
    image_path=$(echo "$line" | grep -oP 'image: \K.*')
    monitor_images["$monitor_name"]="$image_path"
  done

  local monitor current_image
  for monitor in "${monitors[@]}"; do
    current_image="${monitor_images[$monitor]-}"
    printf '%s\0icon\x1f%s\x1finfo\x1f%s\n' "$monitor" "$current_image" "$image"
  done
}

select_monitor() {
  select_monitor_with_path "$(extract_image_from_entry "$*")"
}

# ─── Monitor selection (WE) ──────────────────────────────────────────────────

select_monitor_we() {
  local folder
  folder=$(extract_image_from_entry "$*")

  declare -A monitor_we_ids
  if [[ -s "$WE_MONITOR_STATE_FILE" ]]; then
    local m id
    while IFS=$'\t' read -r m id; do
      [[ -n "$m" && -n "$id" ]] && monitor_we_ids["$m"]="$id"
    done < "$WE_MONITOR_STATE_FILE"
  fi

  local monitors
  mapfile -t monitors < <(hyprctl monitors -j | jq -r '.[].name')

  local monitor current_we_id current_preview project_dir
  for monitor in "${monitors[@]}"; do
    current_we_id="${monitor_we_ids[$monitor]-}"
    project_dir=""
    [[ -n "$current_we_id" && -f "$steam_workshop_dir/$current_we_id/project.json" ]] \
      && project_dir="$steam_workshop_dir/$current_we_id"
    if [[ -n "$project_dir" ]]; then
      current_preview=$(jq -r '.preview // ""' "$project_dir/project.json" 2>/dev/null)
      printf '%s\0icon\x1f%s\x1finfo\x1f%s\n' \
        "$monitor" "$project_dir/$current_preview" "$folder"
    else
      printf '%s\0info\x1f%s\n' "$monitor" "$folder"
    fi
  done
}

# ─── Apply (swww) ────────────────────────────────────────────────────────────

apply() {
  local image="$1"
  local monitor_selection="$2"

  if [[ -z "$monitor_selection" ]]; then
    notify-send -u "critical" "${NOTIFY_OPTIONS[@]}" "No Monitor selected."
    exit 1
  fi

  if command -v matugen &>/dev/null; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Matugen detected setting theme."
    matugen image "$image" --source-color-index 0 --lightness-dark 0.0 --lightness-light 0.0 -t scheme-content
  fi

  if command -v makoctl &>/dev/null; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Mako detected Updating."
    "$XDG_CONFIG_HOME/mako/update-theme.sh"
  fi

  if command -v awww &>/dev/null; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Updating awww image."
    awww img "$image" --transition-type center -o "$monitor_selection"
  fi

  exit 0
}

# ─── Apply (WE) ──────────────────────────────────────────────────────────────

_wait_we_gone() {
  local i
  for ((i = 0; i < 50; i++)); do
    pgrep -f linux-wallpaperengine >/dev/null 2>&1 || return 0
    sleep 0.1
  done
  return 1
}

_update_we_monitor_state() {
  local monitor="$1" workshopid="$2" tmp
  tmp=$(mktemp)
  if [[ -s "$WE_MONITOR_STATE_FILE" ]]; then
    awk -F'\t' -v m="$monitor" '$1 != m' "$WE_MONITOR_STATE_FILE" > "$tmp" 2>/dev/null || true
  fi
  printf '%s\t%s\n' "$monitor" "$workshopid" >> "$tmp"
  mv "$tmp" "$WE_MONITOR_STATE_FILE"
}

apply_we() {
  local folder="$1"
  local monitor_selection="$2"

  if [[ -z "$monitor_selection" ]]; then
    notify-send -u "critical" "${NOTIFY_OPTIONS[@]}" "No Monitor selected."
    exit 1
  fi

  local workshopid="${folder%/}"
  workshopid="${workshopid##*/}"

  local preview_image
  preview_image=$(jq -r '.preview // ""' "$folder/project.json" 2>/dev/null)

  pkill -f linux-wallpaperengine 2>/dev/null || true
  _wait_we_gone || true

  setsid uwsm app -- linux-wallpaperengine \
    --screenshot-delay 1000 \
    --disable-web-security \
    --autoplay-policy=no-user-gesture-required \
    --no-audio-processing \
    --disable-parallax \
    --screenshot "$HOME/.cache/lock.jpg" \
    --bg "$workshopid" \
    --silent \
    --no-fullscreen-pause \
    --scaling fill \
    --screen-root "$monitor_selection" >/dev/null 2>&1 < /dev/null &
  disown 2>/dev/null || true

  sleep 1
  if ! pgrep -f linux-wallpaper >/dev/null 2>&1; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Wallpaper Engine failed to start for ID: $workshopid"
    exit 1
  fi

  _update_we_monitor_state "$monitor_selection" "$workshopid"

  if command -v matugen &>/dev/null && [[ -n "$preview_image" && -f "$folder$preview_image" ]]; then
    notify-send "${NOTIFY_OPTIONS[@]}" "Matugen detected setting theme."
    matugen image "$folder$preview_image" --source-color-index 0 --lightness-dark 0.0 --lightness-light 0.0 -t scheme-content
  fi

  exit 0
}

# ─── ROFI callbacks ──────────────────────────────────────────────────────────

case "$ROFI_RETV" in
  1)
    if [[ "$TAG_MODE" == "selecting" ]]; then
      if [[ "$ROFI_INFO" == tag:* ]]; then
        set_tag_filter "${ROFI_INFO#tag:}"
        TAG_FILTER="${ROFI_INFO#tag:}"
      fi
      set_tag_mode ""
      TAG_MODE=""
      render_view
      exit 0
    fi
    if [[ "$ROFI_INFO" == "back:" ]]; then
      set_drill_path ""
      DRILL_PATH=""
      render_view
    elif [[ "$ROFI_INFO" == folder:* ]]; then
      set_drill_path "${ROFI_INFO#folder:}"
      DRILL_PATH="${ROFI_INFO#folder:}"
      set_view_mode "all"
      VIEW_MODE="all"
      render_view
    elif [[ "$ROFI_INFO" == image:* ]]; then
      # drill-down entry selected — route to monitor picker with the real path
      select_monitor_with_path "${ROFI_INFO#image:}"
    elif [[ -n "$ROFI_INFO" ]]; then
      # monitor was selected; ROFI_INFO is the bare image/folder path → apply.
      # Run in a backgrounded subshell so rofi closes immediately; the subshell
      # is reparented to init when this script exits.
      if [[ "$PICKER_MODE" == "we" ]]; then
        ( apply_we "$ROFI_INFO" "$selection" ) >/dev/null 2>&1 < /dev/null &
      else
        ( apply "$ROFI_INFO" "$selection" ) >/dev/null 2>&1 < /dev/null &
      fi
      disown 2>/dev/null || true
    else
      # top-level wallpaper or WE entry selected (no info) → monitor picker
      if [[ "$PICKER_MODE" == "we" ]]; then
        select_monitor_we "$selection"
      else
        select_monitor "$selection"
      fi
    fi
    exit 0
    ;;
  10)
    # kb-custom-1: toggle favorite on the highlighted entry
    if [[ "$PICKER_MODE" == "we" ]]; then
      folder=$(extract_image_from_entry "$selection")
      if [[ -n "$folder" && -d "${folder%/}" ]]; then
        toggle_we_favorite "$folder"
      fi
    else
      # drill-down entries carry the full path in ROFI_INFO as "image:{path}"
      if [[ "$ROFI_INFO" == image:* ]]; then
        fav_image="${ROFI_INFO#image:}"
      else
        fav_image=$(extract_image_from_entry "$selection")
      fi
      if [[ -n "$fav_image" && -f "$fav_image" ]]; then
        toggle_favorite "$fav_image"
      fi
    fi
    render_view
    exit 0
    ;;
  11)
    # kb-custom-2: toggle all/favorites view; exits drill-down if active
    set_drill_path ""
    DRILL_PATH=""
    if [[ "$VIEW_MODE" == "favorites" ]]; then
      set_view_mode "all"
      VIEW_MODE="all"
    else
      if [[ "$PICKER_MODE" == "we" ]]; then
        fav_file="$WE_FAVORITES_FILE"
      else
        fav_file="$FAVORITES_FILE"
      fi
      if [[ ! -s "$fav_file" ]]; then
        notify-send "${NOTIFY_OPTIONS[@]}" "No favorites yet — press Alt+1 on a wallpaper to add one."
        render_view
        exit 0
      fi
      set_view_mode "favorites"
      VIEW_MODE="favorites"
    fi
    render_view
    exit 0
    ;;
  12)
    # kb-custom-3: rebuild index for current mode; exits drill-down
    set_drill_path ""
    DRILL_PATH=""
    if [[ "$PICKER_MODE" == "we" ]]; then
      if start_we_refresh_background; then
        notify-send "${NOTIFY_OPTIONS[@]}" "Refreshing WE index in the background. Re-open the picker once it's done."
      fi
    else
      if start_refresh_background; then
        notify-send "${NOTIFY_OPTIONS[@]}" "Refreshing wallpaper index in the background. Re-open the picker once it's done."
      fi
    fi
    render_view
    exit 0
    ;;
  13)
    # kb-custom-4: toggle between wallpaper and WE mode
    if [[ "$PICKER_MODE" == "we" ]]; then
      set_picker_mode "wallpaper"
      PICKER_MODE="wallpaper"
    else
      set_picker_mode "we"
      PICKER_MODE="we"
    fi
    set_view_mode "all";    VIEW_MODE="all"
    set_drill_path "";      DRILL_PATH=""
    set_tag_filter "";      TAG_FILTER=""
    set_tag_mode "";        TAG_MODE=""
    render_view
    exit 0
    ;;
  14)
    # kb-custom-5: toggle tag filter selection
    if [[ "$TAG_MODE" == "selecting" ]]; then
      set_tag_mode ""
      TAG_MODE=""
    else
      set_tag_mode "selecting"
      TAG_MODE="selecting"
    fi
    render_view
    exit 0
    ;;
esac

# Initial invocation: restore persisted picker mode and view mode; reset
# transient state (drill-down, tag-selection mode).
dependencies_check
set_drill_path ""; DRILL_PATH=""
set_tag_mode "";   TAG_MODE=""
if [[ "$PICKER_MODE" == "we" ]]; then
  ensure_we_index || exit 1
else
  ensure_index || exit 1
fi
render_view
