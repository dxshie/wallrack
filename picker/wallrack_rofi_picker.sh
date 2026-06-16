#!/usr/bin/env bash
# Rofi script-mode wrapper around `wallrack`. All indexing, filtering,
# favorites, tag overrides, and state management live in the Rust binary;
# this script is just rofi protocol plumbing.
#
# Hard requirements:
# - rofi
# - wallrack
# Soft requirements (per integration backend, configured in config.toml):
# - hyprland (monitors_cmd default)
# - awww (apply_cmd default for image-based integrations)
# - linux-wallpaperengine (apply_cmd for the `we` integration)
#
# Post-apply theming (matugen, mako, etc.) belongs in `post_apply_hook` in
# config.toml — wallrack runs that hook after every successful apply.
#
# Picker modes (cycled with Alt+1):
#   wallpaper   – plain images from config wallpaper dirs
#   we_image    – images extracted from Wallpaper Engine workshop folders
#   we          – live Wallpaper Engine projects (via linux-wallpaperengine)
#   booru       – live tag search against danbooru-style image boards
#
# Rofi script-mode keybindings:
#   Alt+1  kb-custom-1   cycle integration (wallpaper → we_image → we → booru)
#   Alt+2  kb-custom-2   open tag filter selection  (booru: prompt for a new search query)
#   Alt+3  kb-custom-3   toggle favorite on the highlighted entry
#   Alt+4  kb-custom-4   switch between all wallpapers and favorites view
#   Alt+5  kb-custom-5   edit tags on the highlighted entry (add/remove)
#   Alt+6  kb-custom-6   cycle rating filter (All → Mature → Questionable → Everyone)
#                        (booru: cycle search site instead)
#   Alt+7  kb-custom-7   booru: previous page
#   Alt+8  kb-custom-8   booru: next page
#   Alt+0  kb-custom-10  rebuild the index (current integration; no-op for booru)
#
# Rebinding the booru pagination keys to Ctrl+P / Ctrl+N is one rofi flag:
#   rofi -kb-custom-7 "Control+p" -kb-custom-8 "Control+n" \
#        -modi wallpaper:wallrack-rofi-picker -show wallpaper

set -o pipefail

NOTIFY_OPTIONS=(-i "$DOTFILES/logos/we.png" "Wallrack")

# Rofi appends the highlighted entry on every re-invocation; we just want
# the last positional arg (the highlighted row's display text). The rating
# filter is no longer launched-with, it's cycled with Alt+6.
selection="${*: -1}"

picker_mode=$(wallrack state get picker_mode 2>/dev/null || echo wallpaper)
tag_mode=$(wallrack state get tag_mode 2>/dev/null || echo "")
drill_path=$(wallrack state get drill_path 2>/dev/null || echo "")
tag_edit_target=$(wallrack state get tag_edit_target 2>/dev/null || echo "")
tag_add_mode=$(wallrack state get tag_add_mode 2>/dev/null || echo "")
booru_search_mode=$(wallrack state get booru_search_mode 2>/dev/null || echo "")

# Trace every invocation so the booru search flow can be reconstructed
# after the fact. Logs all invocations (not just booru) so we can also see
# how the Alt+1 cycle lands.
mkdir -p "$HOME/.cache/wallrack/booru" 2>/dev/null || true
{
  printf '\n[%s] RETV=%q picker_mode=%q booru_search_mode=%q tag_add_mode=%q\n' \
    "$(date +%H:%M:%S)" "$ROFI_RETV" "$picker_mode" "$booru_search_mode" "$tag_add_mode"
  printf '  ROFI_INFO=%q selection=%q argc=%s\n' \
    "$ROFI_INFO" "$selection" "$#"
  i=1; for arg in "$@"; do printf '  arg[%d]=%q\n' "$i" "$arg"; i=$((i+1)); done
} >> "$HOME/.cache/wallrack/booru/last_error.log" 2>/dev/null || true

# ─── helpers ────────────────────────────────────────────────────────────────

extract_image_from_entry() {
  echo "${1##* - }"
}

next_mode() {
  # wallpaper → we_image → we → booru → wallpaper
  case "$1" in
    wallpaper) echo we_image ;;
    we_image)  echo we ;;
    we)        echo booru ;;
    booru)     echo wallpaper ;;
    *)         echo wallpaper ;;
  esac
}

# ─── booru helpers ─────────────────────────────────────────────────────────

booru_current_site() {
  # Single source of truth in the binary — resolves state → config default →
  # first configured site. Avoids re-implementing the resolution rules in
  # bash (jq's alphabetical key ordering picked "danbooru" over the user's
  # actual default and quietly broke searches).
  local s
  s=$(wallrack booru current-site 2>/dev/null)
  [[ -z "$s" ]] && s=konachan
  # Persist so the booru:site cycle has a starting point next time.
  wallrack state set booru_site "$s" >/dev/null
  echo "$s"
}

booru_next_site() {
  local cur="$1"
  # Round-robin through the configured sites.
  local sites
  sites=$(wallrack booru sites --format=json 2>/dev/null | jq -r 'keys[]' 2>/dev/null)
  [[ -z "$sites" ]] && { echo "$cur"; return; }
  local first="" next="" take=""
  while IFS= read -r s; do
    [[ -z "$s" ]] && continue
    [[ -z "$first" ]] && first="$s"
    if [[ "$take" == "1" ]]; then next="$s"; take=""; fi
    [[ "$s" == "$cur" ]] && take=1
  done <<< "$sites"
  [[ -z "$next" ]] && next="$first"
  echo "$next"
}

booru_run_search() {
  # Run a search using the values currently in state and re-render the view.
  local site query page
  site=$(booru_current_site)
  query=$(wallrack state get booru_query 2>/dev/null || echo "")
  page=$(wallrack state get booru_page 2>/dev/null || echo 1)
  if [[ -z "$query" ]]; then
    # No query yet — just render the booru view's empty state.
    wallrack view
    return
  fi
  local err_log="$HOME/.cache/wallrack/booru/last_error.log"
  mkdir -p "$(dirname "$err_log")"
  : > "$err_log"
  local notif_id
  notif_id=$(notify-send --print-id "${NOTIFY_OPTIONS[@]}" "Searching $site for \`$query\` (page $page)…" 2>/dev/null || true)
  # Keep stderr captured so a silent failure can be surfaced — the previous
  # version dropped errors on the floor and made rofi look broken.
  if ! wallrack booru search --site="$site" --tags="$query" --page="$page" --format=json >/dev/null 2>"$err_log"; then
    local err_msg
    err_msg=$(tr -d '\n' < "$err_log" | head -c 200)
    [[ -z "$err_msg" ]] && err_msg="(no stderr — see ~/.cache/wallrack/booru/last_error.log)"
    notify-send --replace-id="$notif_id" -u critical "${NOTIFY_OPTIONS[@]}" "Booru search failed on $site" "$err_msg"
  else
    notify-send --replace-id="$notif_id" "${NOTIFY_OPTIONS[@]}" "$site · \`$query\` · page $page"
  fi
  wallrack view
}

start_refresh_background() {
  setsid wallrack index --integration="$picker_mode" >/dev/null 2>&1 < /dev/null &
  disown 2>/dev/null || true
}

ensure_index() {
  local integration="$1"
  # booru has no on-disk source to index — `wallrack booru search` builds
  # the index on demand from the API. Skip the first-run prompt.
  if [[ "$integration" == "booru" ]]; then
    wallrack index --integration=booru >/dev/null 2>&1 || true
    return 0
  fi
  if ! wallrack state get _index_built_$integration >/dev/null 2>&1; then
    if [[ ! -f "$HOME/.cache/wallrack/$integration/index.json" ]]; then
      local notif_id
      notif_id=$(notify-send --print-id "${NOTIFY_OPTIONS[@]}" "Building $integration index for the first time — please wait..." 2>/dev/null || true)
      WALLRACK_NOTIF_ID="$notif_id" wallrack index --integration="$integration"
      wallrack state set _index_built_$integration 1 >/dev/null
    fi
  fi
}

# Resolve the wallrack entry id under the rofi highlight. Returns empty if
# the row isn't taggable (folder rows, control rows, booru-control rows).
target_id_from_highlight() {
  if [[ "$ROFI_INFO" == folder:* ]]; then
    echo ""
  elif [[ "$ROFI_INFO" == image:* ]]; then
    echo "${ROFI_INFO#image:}"
  elif [[ "$ROFI_INFO" == booru-post:* ]]; then
    echo "${ROFI_INFO#booru-post:}"
  elif [[ "$ROFI_INFO" == back:* || "$ROFI_INFO" == tag:* || "$ROFI_INFO" == tagedit:* ]]; then
    echo ""
  elif [[ -n "$ROFI_INFO" ]]; then
    echo "$ROFI_INFO"
  else
    extract_image_from_entry "$selection"
  fi
}

# Render the per-entry tag editor as rofi script output. The active target
# lives in state (`tag_edit_target`) so it survives the rofi → script
# round-trip across selections.
render_tag_editor() {
  local target label
  target=$(wallrack state get tag_edit_target 2>/dev/null || echo "")
  if [[ -z "$target" ]]; then
    wallrack view
    return
  fi
  label="${target##*/}"

  printf '\0prompt\x1fTags: %s\n' "$label"
  printf '\0use-hot-keys\x1ftrue\n'
  printf '\0message\x1fEnter to remove tag | "+ Add" prompts for a new tag | ← Back\n'
  printf '← Back\0info\x1ftagedit:back\n'
  printf '+ Add tag…\0info\x1ftagedit:add\n'
  while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    printf '%s\0info\x1ftagedit:remove:%s\n' "$tag" "$tag"
  done < <(wallrack tag show --integration="$picker_mode" --id="$target" 2>/dev/null)
}

# Render the "add tag" prompt inside the running rofi. Rofi refuses to
# launch a nested instance, so instead of spawning `rofi -dmenu` we render
# a script-mode view: a Cancel row, every tag the catalog knows about for
# this integration, and rely on rofi's allow-custom default for entirely
# new tags. Selecting an existing tag row passes its name through as
# $selection just like typing one.
render_add_tag_prompt() {
  local target label tag
  target=$(wallrack state get tag_edit_target 2>/dev/null || echo "")
  label="${target##*/}"
  printf '\0prompt\x1fAdd tag to %s\n' "$label"
  printf '\0use-hot-keys\x1ftrue\n'
  printf '\0message\x1fPick a known tag or type a new one — Enter to add, Esc to cancel\n'
  printf '\0no-custom\x1ffalse\n'
  printf '← Cancel\0info\x1ftagedit:cancel\n'
  while IFS= read -r tag; do
    [[ -z "$tag" ]] && continue
    printf '%s\n' "$tag"
  done < <(wallrack tag available --integration="$picker_mode" --format=rofi 2>/dev/null)
}

# ─── apply wrappers ──────────────────────────────────────────────────────────

apply_image() {
  # Used for both wallpaper and we_image integrations.
  local image="$1" monitor="$2" integration="$3"
  if [[ -z "$monitor" ]]; then
    notify-send -u critical "${NOTIFY_OPTIONS[@]}" "No monitor selected."
    exit 1
  fi
  wallrack apply --integration="$integration" --monitor="$monitor" "$image"
  exit 0
}

apply_we() {
  local folder="$1" monitor="$2"
  if [[ -z "$monitor" ]]; then
    notify-send -u critical "${NOTIFY_OPTIONS[@]}" "No monitor selected."
    exit 1
  fi
  wallrack apply --integration=we --monitor="$monitor" "$folder"
  exit 0
}

# ─── ROFI callbacks ─────────────────────────────────────────────────────────

# Submitting a custom-typed booru query (Alt+2 → type → Enter) fires
# RETV=2 in rofi, not RETV=1: with no-custom enabled and a single Cancel
# row that the typed text doesn't match, rofi reports the submit as a
# cancel-with-text rather than a select. Treat any non-empty selection
# while booru_search_mode is on as the query; only an empty selection is
# a real Esc cancel. Same logic applies to the tag-add prompt.
if [[ "$booru_search_mode" == "on" && ( "$ROFI_RETV" == "1" || "$ROFI_RETV" == "2" ) ]]; then
  wallrack state unset booru_search_mode >/dev/null
  new_query=$(printf '%s' "$selection" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
  if [[ -z "$new_query" || "$new_query" == "← Cancel" ]]; then
    wallrack view
    exit 0
  fi
  wallrack state set booru_query "$new_query" >/dev/null
  wallrack state set booru_page 1 >/dev/null
  booru_run_search
  exit 0
fi

if [[ "$tag_add_mode" == "on" && ( "$ROFI_RETV" == "1" || "$ROFI_RETV" == "2" ) ]]; then
  wallrack state unset tag_add_mode >/dev/null
  new_tag=$(printf '%s' "$selection" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
  if [[ -z "$new_tag" || "$new_tag" == "← Cancel" ]]; then
    render_tag_editor
    exit 0
  fi
  if [[ -n "$tag_edit_target" ]]; then
    wallrack tag add --integration="$picker_mode" --id="$tag_edit_target" "$new_tag" >/dev/null
  fi
  render_tag_editor
  exit 0
fi

case "$ROFI_RETV" in
  1)
    # Item selected — route by ROFI_INFO payload.

    # Booru post selection. Search (Alt+2) and site switch (Alt+6) are
    # hotkey-only — no info markers — so this only handles post rows.
    if [[ "$picker_mode" == "booru" ]]; then
      case "$ROFI_INFO" in
        booru-post:*)
          post_id="${ROFI_INFO#booru-post:}"
          # Download in the foreground so the captured stdout path is the
          # truth — only then route into the wallpaper monitor picker so
          # the user can place it on a screen.
          notify-send "${NOTIFY_OPTIONS[@]}" "Downloading $post_id…"
          dest=$(wallrack booru download "$post_id" 2>/dev/null | tail -n1)
          if [[ -z "$dest" || ! -f "$dest" ]]; then
            notify-send -u critical "${NOTIFY_OPTIONS[@]}" "Booru download failed for $post_id."
            wallrack view
            exit 0
          fi
          notify-send "${NOTIFY_OPTIONS[@]}" "Saved $(basename "$dest") — choose a monitor."
          # Switch the picker into wallpaper-apply mode so the monitor row
          # selection below routes into apply_image (which only fires when
          # picker_mode != "we"). The user can Alt+1 back to booru
          # afterwards; the booru search is still cached.
          wallrack state set picker_mode wallpaper >/dev/null
          wallrack monitors --integration=wallpaper --target="$dest"
          exit 0
          ;;
      esac
    fi

    # Tag-add mode: the user is typing a new tag in rofi's search box.
    # `$selection` carries whatever they typed (rofi's allow-custom default).
    # Tag editor: when a target is staked out, selections drive the editor
    # rather than the main picker. Has to come before the normal routing so
    # `image:` rows underneath the editor don't fire the apply flow.
    if [[ -n "$tag_edit_target" ]]; then
      case "$ROFI_INFO" in
        tagedit:back)
          wallrack state unset tag_edit_target >/dev/null
          wallrack view
          exit 0
          ;;
        tagedit:add)
          wallrack state set tag_add_mode on >/dev/null
          render_add_tag_prompt
          exit 0
          ;;
        tagedit:remove:*)
          tag="${ROFI_INFO#tagedit:remove:}"
          wallrack tag remove --integration="$picker_mode" --id="$tag_edit_target" "$tag" >/dev/null
          render_tag_editor
          exit 0
          ;;
        *)
          # Stray selection (shouldn't happen). Drop back to normal view.
          wallrack state unset tag_edit_target >/dev/null
          wallrack view
          exit 0
          ;;
      esac
    fi

    if [[ "$tag_mode" == "selecting" ]]; then
      if [[ "$ROFI_INFO" == tag:* ]]; then
        wallrack state set tag_filter "${ROFI_INFO#tag:}" >/dev/null
      fi
      wallrack state unset tag_mode >/dev/null
      wallrack view
      exit 0
    fi
    if [[ "$ROFI_INFO" == noop:* ]]; then
      # Placeholder row (e.g. empty-state). Re-render the view rather than
      # let rofi close on us.
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
      target="${ROFI_INFO#image:}"
      wallrack monitors --integration="$picker_mode" --target="$target"
    elif [[ -n "$ROFI_INFO" ]]; then
      # Monitor picker selection: ROFI_INFO carries the target (image path
      # for image integrations, folder path for `we`). Apply detached so
      # rofi closes immediately.
      if [[ "$picker_mode" == "we" ]]; then
        ( apply_we "$ROFI_INFO" "$selection" ) >/dev/null 2>&1 < /dev/null &
      else
        ( apply_image "$ROFI_INFO" "$selection" "$picker_mode" ) >/dev/null 2>&1 < /dev/null &
      fi
      disown 2>/dev/null || true
    else
      # Top-level entry without info field — shouldn't happen for indexed
      # integrations, but fall through to monitor picker just in case.
      target=$(extract_image_from_entry "$selection")
      wallrack monitors --integration="$picker_mode" --target="$target"
    fi
    exit 0
    ;;
  10)
    # kb-custom-1 (Alt+1): cycle integration mode.
    new_mode=$(next_mode "$picker_mode")
    wallrack state set picker_mode "$new_mode" >/dev/null
    wallrack state set view_mode all >/dev/null
    wallrack state unset drill_path >/dev/null
    wallrack state unset tag_filter >/dev/null
    wallrack state unset tag_mode >/dev/null
    wallrack state unset tag_edit_target >/dev/null
    wallrack state unset tag_add_mode >/dev/null
    wallrack state unset booru_search_mode >/dev/null
    ensure_index "$new_mode"
    wallrack view
    exit 0
    ;;
  11)
    # kb-custom-2 (Alt+2): toggle tag filter selection. In booru mode the
    # cached index has no useful per-entry tag distribution, so we hijack
    # this key for "prompt for a new search query" instead.
    if [[ "$picker_mode" == "booru" ]]; then
      wallrack state set booru_search_mode on >/dev/null
      wallrack view
      exit 0
    fi
    if [[ "$tag_mode" == "selecting" ]]; then
      wallrack state unset tag_mode >/dev/null
    else
      wallrack state set tag_mode selecting >/dev/null
    fi
    wallrack view
    exit 0
    ;;
  12)
    # kb-custom-3 (Alt+3): toggle favorite on highlighted entry.
    if [[ "$ROFI_INFO" == folder:* ]]; then
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
      current_view=$(wallrack state get view_mode 2>/dev/null || echo all)
      if [[ "$current_view" == "favorites" ]]; then
        fav_count=$(wallrack favorites list --integration="$picker_mode" --format=json | jq 'length' 2>/dev/null || echo 0)
        if [[ "$fav_count" == "0" ]]; then
          wallrack state set view_mode all >/dev/null
          notify-send "${NOTIFY_OPTIONS[@]}" "No favorites left — switched to all wallpapers."
        fi
      fi
    fi
    wallrack view
    exit 0
    ;;
  13)
    # kb-custom-4 (Alt+4): toggle all/favorites view; exits drill-down.
    wallrack state unset drill_path >/dev/null
    current=$(wallrack state get view_mode 2>/dev/null || echo all)
    if [[ "$current" == "favorites" ]]; then
      wallrack state set view_mode all >/dev/null
    else
      fav_count=$(wallrack favorites list --integration="$picker_mode" --format=json | jq 'length' 2>/dev/null || echo 0)
      if [[ "$fav_count" == "0" ]]; then
        notify-send "${NOTIFY_OPTIONS[@]}" "No favorites yet — press Alt+3 on a wallpaper to add one."
        wallrack view
        exit 0
      fi
      wallrack state set view_mode favorites >/dev/null
    fi
    wallrack view
    exit 0
    ;;
  14)
    # kb-custom-5 (Alt+5): open the tag editor for the highlighted entry,
    # or close it if it's already open (acts as a cancel).
    if [[ -n "$tag_edit_target" ]]; then
      wallrack state unset tag_edit_target >/dev/null
      wallrack state unset tag_add_mode >/dev/null
      wallrack view
      exit 0
    fi
    if [[ "$tag_mode" == "selecting" ]]; then
      # The tag-filter view's rows aren't entries — nothing to edit.
      wallrack view
      exit 0
    fi
    target=$(target_id_from_highlight)
    if [[ -z "$target" ]]; then
      notify-send "${NOTIFY_OPTIONS[@]}" "Highlight a wallpaper to edit its tags."
      wallrack view
      exit 0
    fi
    wallrack state set tag_edit_target "$target" >/dev/null
    render_tag_editor
    exit 0
    ;;
  15)
    # kb-custom-6 (Alt+6): cycle the rating filter
    # All → Mature → Questionable → Everyone → All.
    # In booru mode this key cycles the search site instead — rating-based
    # filtering on a single search page isn't useful (every page is one site
    # already), but jumping sites without losing the query is.
    if [[ "$picker_mode" == "booru" ]]; then
      cur_site=$(booru_current_site)
      new_site=$(booru_next_site "$cur_site")
      wallrack state set booru_site "$new_site" >/dev/null
      wallrack state set booru_page 1 >/dev/null
      notify-send "${NOTIFY_OPTIONS[@]}" "Booru site: $new_site"
      booru_run_search
      exit 0
    fi
    current=$(wallrack state get rating 2>/dev/null || echo All)
    case "$current" in
      All)          next=Mature ;;
      Mature)       next=Questionable ;;
      Questionable) next=Everyone ;;
      Everyone)     next=All ;;
      *)            next=All ;;
    esac
    wallrack state set rating "$next" >/dev/null
    notify-send "${NOTIFY_OPTIONS[@]}" "Rating filter: $next"
    wallrack view
    exit 0
    ;;
  16)
    # kb-custom-7 (Alt+7 by default; user-rebindable to Ctrl+P): booru
    # previous page. No-op outside booru mode.
    if [[ "$picker_mode" == "booru" ]]; then
      query=$(wallrack state get booru_query 2>/dev/null || echo "")
      if [[ -z "$query" ]]; then
        notify-send "${NOTIFY_OPTIONS[@]}" "No active search — press Alt+2 to enter a query first."
        wallrack view
        exit 0
      fi
      page=$(wallrack state get booru_page 2>/dev/null || echo 1)
      (( page > 1 )) && page=$(( page - 1 ))
      wallrack state set booru_page "$page" >/dev/null
      booru_run_search
      exit 0
    fi
    wallrack view
    exit 0
    ;;
  17)
    # kb-custom-8 (Alt+8 by default; user-rebindable to Ctrl+N): booru
    # next page.
    if [[ "$picker_mode" == "booru" ]]; then
      query=$(wallrack state get booru_query 2>/dev/null || echo "")
      if [[ -z "$query" ]]; then
        notify-send "${NOTIFY_OPTIONS[@]}" "No active search — press Alt+2 to enter a query first."
        wallrack view
        exit 0
      fi
      page=$(wallrack state get booru_page 2>/dev/null || echo 1)
      page=$(( page + 1 ))
      wallrack state set booru_page "$page" >/dev/null
      booru_run_search
      exit 0
    fi
    wallrack view
    exit 0
    ;;
  19)
    # kb-custom-10 (Alt+0): rebuild index for current integration. For booru
    # this just re-runs the cached search blocking — there's no on-disk
    # source to walk.
    if [[ "$picker_mode" == "booru" ]]; then
      booru_run_search
      exit 0
    fi
    wallrack state unset drill_path >/dev/null
    start_refresh_background
    notify-send "${NOTIFY_OPTIONS[@]}" "Refreshing $picker_mode index in the background. Re-open the picker once done."
    wallrack view
    exit 0
    ;;
esac

# Initial invocation: clear transient state, ensure index exists, render.
wallrack state reset-transient >/dev/null
ensure_index "$picker_mode" || exit 1
wallrack view
