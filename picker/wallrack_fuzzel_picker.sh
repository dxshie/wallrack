#!/usr/bin/env bash
# Fuzzel-driven wallrack picker. Drives `fuzzel --dmenu --index`: each row is
# a `<label>\0icon\x1f<path>` line fed to fuzzel's stdin, with a parallel
# payload array indexed by fuzzel's `--index` output. Stdin order is
# preserved, absolute thumbnail paths render directly, and there's no MRU
# cache reordering.
#
# Layout follows the wofi wrapper: fuzzel has no rofi-style script
# keybindings, so global actions (`⊕ mode: …`, `⊕ view: …`, …) live as
# header rows at the top and per-entry actions open a sub-menu. Payload
# prefixes (`image:`, `folder:`, `tag:`, `back:`, `tagedit:*`, `action:*`,
# `noop:*`) match the rofi/wofi scripts verbatim.
#
# Hard requirements: fuzzel (>=1.8.2 for absolute icon paths), wallrack, jq.
# Soft requirements: hyprland, swww, linux-wallpaperengine, notify-send,
# mako, matugen.

set -o pipefail

NOTIFY_OPTIONS=(-i "${WALLRACK_NOTIFY_ICON:-dialog-information}" "Wallrack")

FUZZEL_BIN="${FUZZEL_BIN:-fuzzel}"
# --dmenu  : read items from stdin
# --index  : print the picked row's index, not its text — gives us back the
#            payload no matter what (display) text the row showed
# --counter: shows N/M alongside the prompt, useful for big grids
FUZZEL_BASE_ARGS=(--dmenu --index --counter)

TMPDIR="${XDG_RUNTIME_DIR:-/tmp}/wallrack-fuzzel-$$"
mkdir -p "$TMPDIR"
trap 'rm -rf "$TMPDIR"' EXIT

# ─── helpers ────────────────────────────────────────────────────────────────

next_mode() {
  case "$1" in
    wallpaper) echo we_image ;;
    we_image)  echo we ;;
    we)        echo wallpaper ;;
    *)         echo wallpaper ;;
  esac
}

next_rating() {
  case "$1" in
    All)          echo Mature ;;
    Mature)       echo Questionable ;;
    Questionable) echo Everyone ;;
    Everyone)     echo All ;;
    *)            echo All ;;
  esac
}

ensure_index() {
  local integration="$1"
  if ! wallrack state get _index_built_$integration >/dev/null 2>&1; then
    if [[ ! -f "$HOME/.cache/wallrack/$integration/index.json" ]]; then
      notify-send "${NOTIFY_OPTIONS[@]}" "Building $integration index for the first time…" 2>/dev/null || true
      wallrack index --integration="$integration"
      wallrack state set _index_built_$integration 1 >/dev/null
    fi
  fi
}

# Drive fuzzel against `display\tICON\tPAYLOAD` lines on stdin. Returns the
# selected row's payload (third column) on stdout, or non-zero on cancel.
#
# Rows arrive in walker TSV form. Fuzzel's dmenu icon syntax embeds a NUL
# byte (`<label>\0icon\x1f<path>`), but bash variables truncate at NUL, so
# we stream each line straight to a temp file via printf's format escapes
# and never let bash hold the assembled string. The `payloads` array runs
# in parallel and is looked up by fuzzel's `--index` output.
fuzzel_pick() {
  local prompt="$1"
  local input="$TMPDIR/fuzzel-input-$RANDOM"
  : > "$input"
  local -a payloads=()
  local line label icon payload rest
  # Read each TSV line whole, then split by parameter expansion. `read -r
  # IFS=$'\t' a b c` would collapse empty middle fields because tab is an
  # IFS-whitespace character, which would lose the icon-vs-payload
  # distinction for header rows (which have an empty icon column).
  while IFS= read -r line; do
    label="${line%%$'\t'*}"
    rest="${line#*$'\t'}"
    icon="${rest%%$'\t'*}"
    payload="${rest#*$'\t'}"
    payloads+=("$payload")
    if [[ -n "$icon" ]]; then
      # %s prints the bash string literally; \0 and \x1f in the format are
      # interpreted by printf and emit raw NUL / 0x1F bytes — exactly the
      # protocol fuzzel parses in dmenu mode.
      printf '%s\0icon\x1f%s\n' "$label" "$icon" >> "$input"
    else
      printf '%s\n' "$label" >> "$input"
    fi
  done
  if [[ ! -s "$input" ]]; then
    return 1
  fi
  local idx
  idx=$("$FUZZEL_BIN" "${FUZZEL_BASE_ARGS[@]}" --prompt "$prompt> " < "$input")
  if [[ -z "$idx" ]]; then
    return 1
  fi
  printf '%s' "${payloads[$idx]}"
}

# Emit one walker-TSV row to stdout (header / sub-menu use).
emit_action_row() {
  # label, payload — no icon.
  printf '%s\t\t%s\n' "$1" "$2"
}

# Modal action header — kept in sync with the wofi wrapper.
header_rows() {
  local mode="$1" view="$2" tag="$3" rating="$4" drill="$5"
  emit_action_row "⊕ mode: $mode → cycle"                "action:cycle_mode"
  emit_action_row "⊕ view: $view → toggle"               "action:cycle_view"
  emit_action_row "⊕ tag: ${tag:-(none)} → choose"       "action:tag_select"
  emit_action_row "⊕ rating: $rating → cycle"            "action:cycle_rating"
  emit_action_row "⊕ refresh $mode index"                "action:refresh"
  if [[ -n "$drill" ]]; then
    emit_action_row "⊕ exit drill ($(basename "$drill"))" "action:exit_drill"
  fi
}

# Per-entry sub-menu — opens after the user picks an entry.
entry_action_menu() {
  local target="$1"
  local label="${target##*/}"
  local fav_lbl="add favorite"
  if wallrack favorites is --integration="$picker_mode" "$target" 2>/dev/null; then
    fav_lbl="remove favorite"
  fi
  {
    emit_action_row "✓ apply to monitor…" "action:apply"
    emit_action_row "★ $fav_lbl"          "action:fav"
    emit_action_row "# edit tags"         "action:tag_edit"
    emit_action_row "← cancel"            "action:cancel"
  } | fuzzel_pick "$label"
}

# Build a monitor picker on the fly. The walker writer puts target in the
# payload column for every monitor row, so all rows would look identical to
# `--index` consumers — we'd lose which monitor was picked. JSON gives us
# `name` and `current_icon` per monitor, which is exactly what we need.
pick_monitor() {
  local target="$1" mode="$2"
  local sel
  sel=$(
    wallrack monitors --integration="$mode" --target="$target" --format=json \
      | jq -r '.[] | [.name, (.current_icon // ""), .name] | @tsv' \
      | fuzzel_pick "Monitor"
  ) || return 1
  printf '%s' "$sel"
}

# ─── apply wrappers (with theming, mirrors rofi/wofi references) ────────────

apply_image() {
  local image="$1" monitor="$2" integration="$3"
  if command -v matugen &>/dev/null; then
    matugen image "$image" --source-color-index 0 --lightness-dark 0.0 --lightness-light 0.0 -t scheme-content 2>/dev/null
  fi
  local mako_script="${XDG_CONFIG_HOME:-$HOME/.config}/mako/update-theme.sh"
  if command -v makoctl &>/dev/null && [[ -x "$mako_script" ]]; then
    "$mako_script"
  fi
  wallrack apply --integration="$integration" --monitor="$monitor" "$image"
}

apply_we() {
  local folder="$1" monitor="$2"
  wallrack apply --integration=we --monitor="$monitor" "$folder"
  if command -v matugen &>/dev/null; then
    local preview
    preview=$(jq -r '.preview // ""' "$folder/project.json" 2>/dev/null)
    if [[ -n "$preview" && -f "$folder/$preview" ]]; then
      matugen image "$folder/$preview" --source-color-index 0 --lightness-dark 0.0 --lightness-light 0.0 -t scheme-content 2>/dev/null
    fi
  fi
}

# ─── main loop ──────────────────────────────────────────────────────────────

wallrack state reset-transient >/dev/null

picker_mode=$(wallrack state get picker_mode 2>/dev/null || echo wallpaper)
ensure_index "$picker_mode" || exit 1

while true; do
  picker_mode=$(wallrack state get picker_mode 2>/dev/null || echo wallpaper)
  view_mode=$(wallrack state get view_mode 2>/dev/null || echo all)
  tag_filter=$(wallrack state get tag_filter 2>/dev/null || echo "")
  rating=$(wallrack state get rating 2>/dev/null || echo All)
  drill=$(wallrack state get drill_path 2>/dev/null || echo "")
  tag_edit_target=$(wallrack state get tag_edit_target 2>/dev/null || echo "")
  tag_add_mode=$(wallrack state get tag_add_mode 2>/dev/null || echo "")
  tag_mode=$(wallrack state get tag_mode 2>/dev/null || echo "")

  prompt="$picker_mode"
  [[ "$view_mode" == "favorites" ]] && prompt="★ $prompt"
  [[ -n "$tag_filter" ]] && prompt="$prompt #$tag_filter"
  [[ -n "$rating" && "$rating" != "All" ]] && prompt="$prompt ($rating)"

  if [[ "$tag_add_mode" == "on" || -n "$tag_edit_target" || "$tag_mode" == "selecting" ]]; then
    # Sub-views: render exactly what wallrack emits, no extra header — same
    # logic as the wofi wrapper to avoid tempting the user into a mode
    # change mid-edit.
    selection=$(wallrack view --format=walker | fuzzel_pick "$prompt") || selection=""
  else
    selection=$(
      { header_rows "$picker_mode" "$view_mode" "$tag_filter" "$rating" "$drill"
        wallrack view --format=walker
      } | fuzzel_pick "$prompt"
    ) || selection=""
  fi

  if [[ -z "$selection" ]]; then
    # Esc/cancel — back out of any modal sub-view, or exit at the top level.
    if [[ "$tag_add_mode" == "on" ]]; then
      wallrack state unset tag_add_mode >/dev/null
      continue
    fi
    if [[ -n "$tag_edit_target" ]]; then
      wallrack state unset tag_edit_target >/dev/null
      wallrack state unset tag_add_mode >/dev/null
      continue
    fi
    if [[ "$tag_mode" == "selecting" ]]; then
      wallrack state unset tag_mode >/dev/null
      continue
    fi
    exit 0
  fi

  payload="$selection"

  # Tag-add mode: the user picked a catalog tag from the menu. fuzzel
  # `--index` mode echoes only the row index back, so creating a brand-new
  # tag (no matching catalog row) isn't reachable from here — use the rofi
  # picker for that, or pre-create the tag via `wallrack tag create`.
  if [[ "$tag_add_mode" == "on" ]]; then
    wallrack state unset tag_add_mode >/dev/null
    if [[ "$payload" == "tagedit:cancel" ]]; then
      continue
    fi
    if [[ "$payload" == tagedit:pick:* ]]; then
      new_tag="${payload#tagedit:pick:}"
      if [[ -n "$new_tag" && -n "$tag_edit_target" ]]; then
        wallrack tag add --integration="$picker_mode" --id="$tag_edit_target" "$new_tag" >/dev/null
      fi
    fi
    continue
  fi

  if [[ -n "$tag_edit_target" ]]; then
    case "$payload" in
      tagedit:back)
        wallrack state unset tag_edit_target >/dev/null ;;
      tagedit:add)
        wallrack state set tag_add_mode on >/dev/null ;;
      tagedit:remove:*)
        wallrack tag remove --integration="$picker_mode" --id="$tag_edit_target" "${payload#tagedit:remove:}" >/dev/null ;;
      *)
        wallrack state unset tag_edit_target >/dev/null ;;
    esac
    continue
  fi

  if [[ "$tag_mode" == "selecting" ]]; then
    if [[ "$payload" == tag:* ]]; then
      wallrack state set tag_filter "${payload#tag:}" >/dev/null
    fi
    wallrack state unset tag_mode >/dev/null
    continue
  fi

  case "$payload" in
    action:cycle_mode)
      new_mode=$(next_mode "$picker_mode")
      wallrack state set picker_mode "$new_mode" >/dev/null
      wallrack state set view_mode all >/dev/null
      wallrack state unset drill_path >/dev/null
      wallrack state unset tag_filter >/dev/null
      wallrack state unset tag_mode >/dev/null
      ensure_index "$new_mode"
      ;;
    action:cycle_view)
      wallrack state unset drill_path >/dev/null
      if [[ "$view_mode" == "favorites" ]]; then
        wallrack state set view_mode all >/dev/null
      else
        fav_count=$(wallrack favorites list --integration="$picker_mode" --format=json | jq 'length' 2>/dev/null || echo 0)
        if [[ "$fav_count" == "0" ]]; then
          notify-send "${NOTIFY_OPTIONS[@]}" "No favorites yet — favorite an entry first." 2>/dev/null || true
        else
          wallrack state set view_mode favorites >/dev/null
        fi
      fi
      ;;
    action:tag_select)
      wallrack state set tag_mode selecting >/dev/null
      ;;
    action:cycle_rating)
      new=$(next_rating "$rating")
      wallrack state set rating "$new" >/dev/null
      notify-send "${NOTIFY_OPTIONS[@]}" "Rating filter: $new" 2>/dev/null || true
      ;;
    action:refresh)
      wallrack state unset drill_path >/dev/null
      setsid wallrack index --integration="$picker_mode" >/dev/null 2>&1 < /dev/null &
      disown 2>/dev/null || true
      notify-send "${NOTIFY_OPTIONS[@]}" "Refreshing $picker_mode index — re-open the picker once done." 2>/dev/null || true
      exit 0
      ;;
    action:exit_drill)
      wallrack state unset drill_path >/dev/null
      ;;
    back:)
      wallrack state unset drill_path >/dev/null
      ;;
    folder:*)
      wallrack state set drill_path "${payload#folder:}" >/dev/null
      wallrack state set view_mode all >/dev/null
      ;;
    noop:*)
      : ;;
    image:*)
      target="${payload#image:}"
      sub=$(entry_action_menu "$target")
      case "$sub" in
        action:apply)
          mon=$(pick_monitor "$target" "$picker_mode") || continue
          if [[ "$picker_mode" == "we" ]]; then
            ( apply_we "$target" "$mon" ) >/dev/null 2>&1 < /dev/null &
          else
            ( apply_image "$target" "$mon" "$picker_mode" ) >/dev/null 2>&1 < /dev/null &
          fi
          disown 2>/dev/null || true
          exit 0
          ;;
        action:fav)
          result=$(wallrack favorites toggle --integration="$picker_mode" "$target")
          notify-send "${NOTIFY_OPTIONS[@]}" "$result favorite: ${target##*/}" 2>/dev/null || true
          if [[ "$view_mode" == "favorites" ]]; then
            fav_count=$(wallrack favorites list --integration="$picker_mode" --format=json | jq 'length' 2>/dev/null || echo 0)
            if [[ "$fav_count" == "0" ]]; then
              wallrack state set view_mode all >/dev/null
            fi
          fi
          ;;
        action:tag_edit)
          wallrack state set tag_edit_target "$target" >/dev/null
          ;;
        action:cancel|"")
          ;;
      esac
      ;;
    *)
      : ;;
  esac
done
