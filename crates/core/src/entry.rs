//! Indexed entries — one variant per source kind. Replaces the previous flat
//! struct that overloaded `workshop_id` (root dir / workshop id / download
//! URL) and `subfolder` (in-project path / booru site name) across four
//! distinct integrations.
//!
//! Each variant matches exactly one
//! [`Integration`][crate::integrations::Integration] impl, so the variant
//! tag IS the integration key. Accessors below expose the common surface
//! (id, title, thumb, tags, rating, source) every renderer and filter
//! expects; per-variant typed accessors handle the rest.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A single indexed item. Tagged JSON shape uses
/// `kind = "image" | "we_image" | "project" | "booru_post"` as the
/// discriminator; serde derive handles the (de)serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    /// A plain wallpaper image from one of `wallpaper.dirs`. `root` is the
    /// configured directory the image was found under — used as a grouping
    /// discriminator so two configured dirs sharing a subfolder name don't
    /// collide in the picker.
    Image {
        id: String,
        title: String,
        source: PathBuf,
        thumb: PathBuf,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        rating: String,
        #[serde(default)]
        subfolder: String,
        root: PathBuf,
    },
    /// An image extracted from a Wallpaper Engine workshop project.
    /// `workshop_id` is the parent project id; `project_root` is the
    /// workshop folder root used for `strip_prefix` to compute `subfolder`.
    WeImage {
        id: String,
        title: String,
        source: PathBuf,
        thumb: PathBuf,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        rating: String,
        #[serde(default)]
        subfolder: String,
        workshop_id: String,
        project_root: PathBuf,
    },
    /// A live Wallpaper Engine project applied via `linux-wallpaperengine`.
    Project {
        id: String,
        title: String,
        folder: PathBuf,
        thumb: PathBuf,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        rating: String,
        workshop_id: String,
    },
    /// A booru post from a cached search. `download_url` is what
    /// `BooruIntegration::apply` downloads; `predicted_path` is the
    /// destination path under `[booru].download_dir`.
    BooruPost {
        /// `"<site>:<post_id>"` slug.
        id: String,
        site: String,
        post_id: u64,
        title: String,
        thumb: PathBuf,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        rating: String,
        download_url: String,
        predicted_path: PathBuf,
    },
}

impl Entry {
    pub fn integration(&self) -> &'static str {
        match self {
            Self::Image { .. } => "wallpaper",
            Self::WeImage { .. } => "we_image",
            Self::Project { .. } => "we",
            Self::BooruPost { .. } => "booru",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Image { id, .. }
            | Self::WeImage { id, .. }
            | Self::Project { id, .. }
            | Self::BooruPost { id, .. } => id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Image { title, .. }
            | Self::WeImage { title, .. }
            | Self::Project { title, .. }
            | Self::BooruPost { title, .. } => title,
        }
    }

    pub fn thumb(&self) -> &Path {
        match self {
            Self::Image { thumb, .. }
            | Self::WeImage { thumb, .. }
            | Self::Project { thumb, .. }
            | Self::BooruPost { thumb, .. } => thumb,
        }
    }

    pub fn set_thumb(&mut self, p: PathBuf) {
        match self {
            Self::Image { thumb, .. }
            | Self::WeImage { thumb, .. }
            | Self::Project { thumb, .. }
            | Self::BooruPost { thumb, .. } => *thumb = p,
        }
    }

    pub fn tags(&self) -> &[String] {
        match self {
            Self::Image { tags, .. }
            | Self::WeImage { tags, .. }
            | Self::Project { tags, .. }
            | Self::BooruPost { tags, .. } => tags,
        }
    }

    pub fn tags_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::Image { tags, .. }
            | Self::WeImage { tags, .. }
            | Self::Project { tags, .. }
            | Self::BooruPost { tags, .. } => tags,
        }
    }

    pub fn set_tags(&mut self, new: Vec<String>) {
        *self.tags_mut() = new;
    }

    pub fn rating(&self) -> &str {
        match self {
            Self::Image { rating, .. }
            | Self::WeImage { rating, .. }
            | Self::Project { rating, .. }
            | Self::BooruPost { rating, .. } => rating,
        }
    }

    pub fn set_rating(&mut self, r: String) {
        match self {
            Self::Image { rating, .. }
            | Self::WeImage { rating, .. }
            | Self::Project { rating, .. }
            | Self::BooruPost { rating, .. } => *rating = r,
        }
    }

    /// Path on disk for the entry's primary asset. For image entries this
    /// is the image file; for `Project` it's the project folder; for
    /// `BooruPost` it's the predicted download destination.
    pub fn source(&self) -> &Path {
        match self {
            Self::Image { source, .. } | Self::WeImage { source, .. } => source,
            Self::Project { folder, .. } => folder,
            Self::BooruPost { predicted_path, .. } => predicted_path,
        }
    }

    /// Subfolder relative to the entry's grouping root. `None` for entries
    /// where drilling doesn't apply (`Project`, `BooruPost`).
    pub fn subfolder(&self) -> Option<&str> {
        match self {
            Self::Image { subfolder, .. } | Self::WeImage { subfolder, .. } => Some(subfolder),
            _ => None,
        }
    }

    /// Workshop project id where applicable. Returned for `Project` (the
    /// live WE project id) and `WeImage` (the parent project's id).
    pub fn workshop_id(&self) -> Option<&str> {
        match self {
            Self::WeImage { workshop_id, .. } | Self::Project { workshop_id, .. } => {
                Some(workshop_id)
            }
            _ => None,
        }
    }

    /// Stable grouping discriminator used by `emit_grouped_view`. Two
    /// entries with the same `(group_key, subfolder)` collapse into one
    /// folder row.
    pub fn group_key(&self) -> String {
        match self {
            Self::Image { root, .. } => root.to_string_lossy().into_owned(),
            Self::WeImage { workshop_id, .. } => workshop_id.clone(),
            Self::Project { workshop_id, .. } => workshop_id.clone(),
            Self::BooruPost { site, .. } => site.clone(),
        }
    }

    /// For `BooruPost` only — the full-size download URL `apply` fetches.
    pub fn download_url(&self) -> Option<&str> {
        match self {
            Self::BooruPost { download_url, .. } => Some(download_url),
            _ => None,
        }
    }
}

/// The cached index for a single integration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub integration: String,
    pub entries: Vec<Entry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Entry {
        Entry::Image {
            id: "/wp/a.jpg".into(),
            title: "a".into(),
            source: PathBuf::from("/wp/a.jpg"),
            thumb: PathBuf::from("/cache/a.jpg"),
            tags: vec!["x".into()],
            rating: "Everyone".into(),
            subfolder: "sub".into(),
            root: PathBuf::from("/wp"),
        }
    }

    fn we_image() -> Entry {
        Entry::WeImage {
            id: "we:123:b".into(),
            title: "b".into(),
            source: PathBuf::from("/we/123/b.jpg"),
            thumb: PathBuf::from("/cache/b.jpg"),
            tags: vec![],
            rating: String::new(),
            subfolder: "inner".into(),
            workshop_id: "123".into(),
            project_root: PathBuf::from("/we/123"),
        }
    }

    fn project() -> Entry {
        Entry::Project {
            id: "456".into(),
            title: "p".into(),
            folder: PathBuf::from("/we/456"),
            thumb: PathBuf::from("/cache/p.jpg"),
            tags: vec![],
            rating: String::new(),
            workshop_id: "456".into(),
        }
    }

    fn booru_post() -> Entry {
        Entry::BooruPost {
            id: "konachan:42".into(),
            site: "konachan".into(),
            post_id: 42,
            title: "k42".into(),
            thumb: PathBuf::from("/cache/k42.jpg"),
            tags: vec!["scenery".into()],
            rating: "s".into(),
            download_url: "https://konachan.com/full.jpg".into(),
            predicted_path: PathBuf::from("/downloads/k42.jpg"),
        }
    }

    #[test]
    fn integration_key_matches_variant() {
        assert_eq!(image().integration(), "wallpaper");
        assert_eq!(we_image().integration(), "we_image");
        assert_eq!(project().integration(), "we");
        assert_eq!(booru_post().integration(), "booru");
    }

    #[test]
    fn source_returns_kind_specific_path() {
        assert_eq!(image().source(), Path::new("/wp/a.jpg"));
        assert_eq!(we_image().source(), Path::new("/we/123/b.jpg"));
        assert_eq!(project().source(), Path::new("/we/456"));
        // BooruPost surfaces the predicted destination, not the URL.
        assert_eq!(booru_post().source(), Path::new("/downloads/k42.jpg"));
    }

    #[test]
    fn subfolder_only_for_image_kinds() {
        assert_eq!(image().subfolder(), Some("sub"));
        assert_eq!(we_image().subfolder(), Some("inner"));
        assert_eq!(project().subfolder(), None);
        assert_eq!(booru_post().subfolder(), None);
    }

    #[test]
    fn workshop_id_only_for_we_variants() {
        assert_eq!(image().workshop_id(), None);
        assert_eq!(we_image().workshop_id(), Some("123"));
        assert_eq!(project().workshop_id(), Some("456"));
        assert_eq!(booru_post().workshop_id(), None);
    }

    #[test]
    fn download_url_only_for_booru_post() {
        assert_eq!(
            booru_post().download_url(),
            Some("https://konachan.com/full.jpg")
        );
        assert_eq!(image().download_url(), None);
        assert_eq!(project().download_url(), None);
    }

    #[test]
    fn group_key_is_stable_per_variant() {
        assert_eq!(image().group_key(), "/wp");
        assert_eq!(we_image().group_key(), "123");
        assert_eq!(project().group_key(), "456");
        assert_eq!(booru_post().group_key(), "konachan");
    }

    #[test]
    fn set_tags_replaces_collection_for_all_variants() {
        for mut e in [image(), we_image(), project(), booru_post()] {
            e.set_tags(vec!["new".into()]);
            assert_eq!(e.tags(), &["new"]);
        }
    }

    #[test]
    fn serde_tag_is_snake_case_kind() {
        let json = serde_json::to_value(image()).unwrap();
        assert_eq!(json.get("kind").and_then(|v| v.as_str()), Some("image"));
        let json = serde_json::to_value(we_image()).unwrap();
        assert_eq!(json.get("kind").and_then(|v| v.as_str()), Some("we_image"));
        let json = serde_json::to_value(project()).unwrap();
        assert_eq!(json.get("kind").and_then(|v| v.as_str()), Some("project"));
        let json = serde_json::to_value(booru_post()).unwrap();
        assert_eq!(
            json.get("kind").and_then(|v| v.as_str()),
            Some("booru_post")
        );
    }

    #[test]
    fn entry_round_trips_through_json() {
        for original in [image(), we_image(), project(), booru_post()] {
            let s = serde_json::to_string(&original).unwrap();
            let parsed: Entry = serde_json::from_str(&s).unwrap();
            assert_eq!(parsed.id(), original.id());
            assert_eq!(parsed.integration(), original.integration());
            assert_eq!(parsed.group_key(), original.group_key());
        }
    }
}
