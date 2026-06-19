//! Picker action protocol — the typed enum that replaces the magic-string
//! `info:` payloads (`image:<id>`, `folder:<path>`, `booru-post:<id>`,
//! `tagedit:add`, `noop:empty`, …) wallrack has historically used.
//!
//! On the wire (rofi `info` field, dmenu payload column, JSON `info` key)
//! the legacy string form is still emitted so the bundled picker shells
//! keep working without changes. The structured form is exposed in JSON
//! output under a new `action` key and in the public Rust API so future
//! frontends and tests can match on the enum directly instead of parsing
//! strings.

use serde::{Deserialize, Serialize};

/// One routing payload attached to a picker row. Each variant maps to a
/// stable legacy string form via [`Action::to_legacy_string`] /
/// [`Action::parse_legacy`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Apply an image entry to a monitor. `id` is the entry id (absolute
    /// path for image integrations, project folder for `we`).
    ApplyImage { id: String },
    /// Drill into a folder row (image integrations only). `folder` is the
    /// folder path with a trailing slash.
    Drill { folder: String },
    /// Leave the current drill view.
    Back,
    /// Download then apply a booru post — `id` is the `<site>:<post_id>` slug.
    BooruPost { id: String },
    /// Booru search-results row: same target as `BooruPost`, distinct slug
    /// used by the search command's direct output.
    BooruSearchHit { id: String },
    /// Cancel the booru search prompt.
    BooruCancelSearch,
    /// Filter the current view by `tag`. Empty string clears the filter
    /// (the "All tags" reset row).
    FilterTag { tag: String },
    /// Leave the tag editor.
    TagEditBack,
    /// Open the add-tag prompt for the current entry.
    TagEditAdd,
    /// Cancel the add-tag prompt.
    TagEditCancel,
    /// Remove `tag` from the current entry.
    TagEditRemove { tag: String },
    /// Pick a catalog tag to add to the current entry.
    TagEditPick { tag: String },
    /// Inert placeholder. `reason` is a short discriminator for the
    /// originating empty-state view.
    Noop { reason: String },
    /// Free-form payload with no semantic kind. Used by the monitor picker
    /// where the picker shell expects a raw string (an apply target or a
    /// monitor name) rather than one of the prefix-tagged actions.
    Raw { value: String },
}

impl Action {
    /// Render to the legacy magic-string form the picker shells consume.
    pub fn to_legacy_string(&self) -> String {
        match self {
            Self::ApplyImage { id } => format!("image:{id}"),
            Self::Drill { folder } => format!("folder:{folder}"),
            Self::Back => "back:".to_string(),
            Self::BooruPost { id } => format!("booru-post:{id}"),
            Self::BooruSearchHit { id } => format!("booru:{id}"),
            Self::BooruCancelSearch => "booru:cancel-search".to_string(),
            Self::FilterTag { tag } => format!("tag:{tag}"),
            Self::TagEditBack => "tagedit:back".to_string(),
            Self::TagEditAdd => "tagedit:add".to_string(),
            Self::TagEditCancel => "tagedit:cancel".to_string(),
            Self::TagEditRemove { tag } => format!("tagedit:remove:{tag}"),
            Self::TagEditPick { tag } => format!("tagedit:pick:{tag}"),
            Self::Noop { reason } => format!("noop:{reason}"),
            Self::Raw { value } => value.clone(),
        }
    }

    /// Inverse of [`to_legacy_string`] — for tests and for Rust callers
    /// receiving an arbitrary legacy payload off the picker.
    pub fn parse_legacy(s: &str) -> Option<Self> {
        if s == "back:" {
            return Some(Self::Back);
        }
        if s == "booru:cancel-search" {
            return Some(Self::BooruCancelSearch);
        }
        if s == "tagedit:back" {
            return Some(Self::TagEditBack);
        }
        if s == "tagedit:add" {
            return Some(Self::TagEditAdd);
        }
        if s == "tagedit:cancel" {
            return Some(Self::TagEditCancel);
        }
        if let Some(id) = s.strip_prefix("image:") {
            return Some(Self::ApplyImage { id: id.to_string() });
        }
        if let Some(folder) = s.strip_prefix("folder:") {
            return Some(Self::Drill {
                folder: folder.to_string(),
            });
        }
        if let Some(id) = s.strip_prefix("booru-post:") {
            return Some(Self::BooruPost { id: id.to_string() });
        }
        // `booru:cancel-search` matched above; remaining `booru:*` is a hit.
        if let Some(id) = s.strip_prefix("booru:") {
            return Some(Self::BooruSearchHit { id: id.to_string() });
        }
        if let Some(tag) = s.strip_prefix("tagedit:remove:") {
            return Some(Self::TagEditRemove { tag: tag.to_string() });
        }
        if let Some(tag) = s.strip_prefix("tagedit:pick:") {
            return Some(Self::TagEditPick { tag: tag.to_string() });
        }
        if let Some(tag) = s.strip_prefix("tag:") {
            return Some(Self::FilterTag { tag: tag.to_string() });
        }
        if let Some(reason) = s.strip_prefix("noop:") {
            return Some(Self::Noop {
                reason: reason.to_string(),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Action;

    #[test]
    fn roundtrips_every_variant() {
        let cases = [
            Action::ApplyImage {
                id: "/path/foo.jpg".into(),
            },
            Action::Drill {
                folder: "/path/sub/".into(),
            },
            Action::Back,
            Action::BooruPost {
                id: "konachan:12345".into(),
            },
            Action::BooruSearchHit {
                id: "konachan:12345".into(),
            },
            Action::BooruCancelSearch,
            Action::FilterTag { tag: "scenery".into() },
            Action::FilterTag { tag: String::new() },
            Action::TagEditBack,
            Action::TagEditAdd,
            Action::TagEditCancel,
            Action::TagEditRemove { tag: "neon".into() },
            Action::TagEditPick { tag: "neon".into() },
            Action::Noop {
                reason: "empty".into(),
            },
        ];
        for a in cases {
            let s = a.to_legacy_string();
            let parsed = Action::parse_legacy(&s).expect(&s);
            assert_eq!(parsed, a, "roundtrip failed for {s}");
        }
    }
}
