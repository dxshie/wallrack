#!/usr/bin/env bash
# Wofi-driven wallrack picker. Wofi has no equivalent of rofi's
# script-mode keybindings, so this wrapper builds a loop: each user action
# re-invokes wofi, and the rofi script's Alt+N keys are replaced by
# action header rows ("⊕ mode: wallpaper → cycle", etc.) at the top of the
# list. Entry actions (apply / favorite / edit tags) live in a sub-menu
# that opens after picking an entry.
#
# Protocol: each wofi row is `[img:THUMB:]DISPLAY\x1fPAYLOAD`. Wofi echoes
# the selected line on stdout; we strip the optional `img:` prefix and split
# the rest on U+001F to recover the payload. Payload prefixes match the
# rofi reference picker (`image:`, `folder:`, `tag:`, `back:`, `tagedit:*`)
# plus an `action:*` family this wrapper invents.
#
# Hard requirements: wofi (>=1.4 for `--allow-images`), wallrack, jq.
# Soft requirements (per-integration backend, same as rofi reference):
# hyprland, swww, linux-wallpaperengine, notify-send, mako, matugen.

set -o pipefail

NOTIFY_OPTIONS=(-i "${WALLRACK_NOTIFY_ICON:-dialog-information}" "Wallrack")

WOFI_BIN="${WOFI_BIN:-wofi}"
WOFI_BASE_ARGS=(--dmenu --allow-images --width 900 --height 700)

# U+001F (unit separator) — same character the binary emits between display
# and payload.
US=$'\x1f'

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

# Extract the payload from a wofi-returned line. Strips a leading `img:PATH:`
# prefix wofi echoes back, then splits on the unit separator.
parse_payload() {
  local line="$1"
  if [[ "$line" == img:*:* ]]; then
    line="${line#img:*:}"
  fi
  local payload="${line#*$US}"
  [[ "$payload" == "$line" ]] && payload=""
  printf '%s' "$payload"
}

# Same as above but for the display portion (everything before US).
parse_display() {
  local line="$1"
  if [[ "$line" == img:*:* ]]; then
    line="${line#img:*:}"
  fi
  printf '%s' "${line%%$US*}"
}

emit_action_row() {
  # label, payload — no icon.
  printf '%s%s%s\n' "$1" "$US" "$2"
}

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

# Per-entry action menu, opened after the user picks an entry. Without
# keybindings we can't directly favorite or open the tag editor on the
# highlighted row like the rofi script does, so we route through this menu.
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
  } | "$WOFI_BIN" "${WOFI_BASE_ARGS[@]}" --prompt "$label"
}

pick_monitor() {
  local target="$1" mode="$2"
  local sel
  sel=$(wallrack monitors --integration="$mode" --target="$target" --format=wofi \
        | "$WOFI_BIN" "${WOFI_BASE_ARGS[@]}" --prompt "Monitor")
  [[ -z "$sel" ]] && return 1
  printf '%s' "$(parse_display "$sel")"
}

# ─── apply wrappers (with theming, mirrors rofi reference) ──────────────────

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
    # Sub-views: render exactly what the binary emits — no action header so
    # the user isn't tempted to change global state mid-edit.
    selection=$(wallrack view --format=wofi | "$WOFI_BIN" "${WOFI_BASE_ARGS[@]}" --prompt "$prompt")
  else
    selection=$(
      { header_rows "$picker_mode" "$view_mode" "$tag_filter" "$rating" "$drill"
        wallrack view --format=wofi
      } | "$WOFI_BIN" "${WOFI_BASE_ARGS[@]}" --prompt "$prompt"
    )
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

  payload=$(parse_payload "$selection")
  display=$(parse_display "$selection")

  # Tag add mode: the user either picked a catalog tag, typed a new one, or
  # selected the cancel row.
  if [[ "$tag_add_mode" == "on" ]]; then
    wallrack state unset tag_add_mode >/dev/null
    if [[ "$payload" == "tagedit:cancel" ]]; then
      continue
    fi
    if [[ "$payload" == tagedit:pick:* ]]; then
      new_tag="${payload#tagedit:pick:}"
    else
      new_tag=$(printf '%s' "$display" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
    fi
    if [[ -n "$new_tag" && -n "$tag_edit_target" ]]; then
      wallrack tag add --integration="$picker_mode" --id="$tag_edit_target" "$new_tag" >/dev/null
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
      sub_payload=$(parse_payload "$sub")
      case "$sub_payload" in
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
          # If we just emptied the favorites view, drop back to all.
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
