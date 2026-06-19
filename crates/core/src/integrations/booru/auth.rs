//! Booru auth-query and URL-encoding helpers. Kept separate from `api`
//! because the auth shape differs per booru family and the percent-encoder
//! is small enough to be self-contained.

use crate::config::{BooruApiKind, BooruSite};

/// Build the per-site auth query suffix. Each booru family wires this
/// differently: moebooru wants `login` + `password_hash`, danbooru wants
/// `login` + `api_key`, gelbooru wants `user_id` + `api_key`. Missing
/// credentials produce an empty suffix (anonymous request).
pub(super) fn auth_query(site: &BooruSite) -> String {
    match site.api_kind {
        BooruApiKind::Moebooru => moebooru_auth(site),
        BooruApiKind::Danbooru => match (&site.login, &site.api_key) {
            (Some(login), Some(key)) => format!(
                "&login={}&api_key={}",
                encode_query(login),
                encode_query(key)
            ),
            _ => String::new(),
        },
        BooruApiKind::Gelbooru => match (&site.user_id, &site.api_key) {
            (Some(uid), Some(key)) => format!(
                "&user_id={}&api_key={}",
                encode_query(uid),
                encode_query(key)
            ),
            _ => String::new(),
        },
    }
}

/// Moebooru wants `login=user&password_hash=hex_sha1`. The hash is
/// `SHA1(salt_template_with_{}_replaced_by_password)`. Each site has its own
/// salt (konachan ≠ yande.re), shipped as a builtin. Users can either
/// stash the plaintext `password` (we hash it) or paste the
/// `password_hash` directly from their browser cookies.
fn moebooru_auth(site: &BooruSite) -> String {
    use sha1::{Digest, Sha1};
    let Some(login) = site.login.as_deref().filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let hash = if let Some(h) = site.password_hash.as_deref().filter(|s| !s.is_empty()) {
        h.to_string()
    } else if let (Some(pw), Some(salt)) = (
        site.password.as_deref().filter(|s| !s.is_empty()),
        site.password_salt.as_deref().filter(|s| !s.is_empty()),
    ) {
        let salted = salt.replacen("{}", pw, 1);
        hex::encode(Sha1::digest(salted.as_bytes()))
    } else {
        log::warn!(
            "moebooru auth: login set but no password/password_hash (or salt missing) — \
             sending anonymous request"
        );
        return String::new();
    };
    format!(
        "&login={}&password_hash={}",
        encode_query(login),
        encode_query(&hash)
    )
}

/// Mask credential-bearing query params so URLs are safe to put in logs and
/// notifications. The booru never returns these values, so masking is
/// one-way — we don't have to round-trip them.
pub(super) fn redact_credentials(url: &str) -> String {
    use std::fmt::Write;
    const SENSITIVE: &[&str] = &["api_key", "password_hash", "password", "login", "user_id"];
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(base);
    out.push('?');
    let mut first = true;
    for kv in query.split('&') {
        if !first {
            out.push('&');
        }
        first = false;
        match kv.split_once('=') {
            Some((k, _)) if SENSITIVE.contains(&k) => {
                let _ = write!(out, "{k}=***");
            }
            _ => out.push_str(kv),
        }
    }
    out
}

/// Bare-bones percent encoder. Booru tag queries are whitespace-separated
/// `a-z0-9_:()-` plus the occasional `*`/`!`/`%`, none of which is reserved
/// in a query string — encoding spaces and a handful of meta chars is enough.
pub(super) fn encode_query(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push('+'),
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "%{b:02X}");
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(kind: BooruApiKind) -> BooruSite {
        BooruSite {
            base_url: "https://example.com".into(),
            api_kind: kind,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: None,
        }
    }

    #[test]
    fn encode_query_preserves_unreserved_chars() {
        assert_eq!(encode_query("abcXYZ012-_.~"), "abcXYZ012-_.~");
    }

    #[test]
    fn encode_query_substitutes_space_with_plus() {
        assert_eq!(encode_query("red sky scenery"), "red+sky+scenery");
    }

    #[test]
    fn encode_query_percent_encodes_special_chars() {
        // `:` and `*` are common in booru tag queries (`rating:safe`, `*`).
        assert_eq!(encode_query("rating:safe"), "rating%3Asafe");
        assert_eq!(encode_query("a*b"), "a%2Ab");
    }

    #[test]
    fn encode_query_percent_encodes_utf8_bytes() {
        // 猫 = E7 8C AB
        assert_eq!(encode_query("猫"), "%E7%8C%AB");
    }

    #[test]
    fn redact_credentials_masks_known_keys() {
        let url = "https://example.com/post.json?tags=cat&login=alice&password_hash=deadbeef&api_key=secret";
        let masked = redact_credentials(url);
        assert!(masked.contains("tags=cat"));
        assert!(masked.contains("login=***"));
        assert!(masked.contains("password_hash=***"));
        assert!(masked.contains("api_key=***"));
        assert!(!masked.contains("alice"));
        assert!(!masked.contains("deadbeef"));
        assert!(!masked.contains("secret"));
    }

    #[test]
    fn redact_credentials_passes_through_url_without_query() {
        let url = "https://example.com/post.json";
        assert_eq!(redact_credentials(url), url);
    }

    #[test]
    fn auth_query_is_empty_when_no_credentials_set() {
        for kind in [
            BooruApiKind::Moebooru,
            BooruApiKind::Danbooru,
            BooruApiKind::Gelbooru,
        ] {
            assert_eq!(auth_query(&site(kind)), "");
        }
    }

    #[test]
    fn auth_query_danbooru_requires_both_login_and_api_key() {
        let mut s = site(BooruApiKind::Danbooru);
        s.login = Some("alice".into());
        // Login alone: no auth.
        assert_eq!(auth_query(&s), "");
        s.api_key = Some("k e y".into());
        // Spaces get plus-encoded by encode_query.
        assert_eq!(auth_query(&s), "&login=alice&api_key=k+e+y");
    }

    #[test]
    fn auth_query_gelbooru_requires_both_user_id_and_api_key() {
        let mut s = site(BooruApiKind::Gelbooru);
        s.user_id = Some("123".into());
        assert_eq!(auth_query(&s), "");
        s.api_key = Some("secret".into());
        assert_eq!(auth_query(&s), "&user_id=123&api_key=secret");
    }

    #[test]
    fn auth_query_moebooru_uses_password_hash_when_present() {
        let mut s = site(BooruApiKind::Moebooru);
        s.login = Some("alice".into());
        s.password_hash = Some("deadbeef".into());
        assert_eq!(auth_query(&s), "&login=alice&password_hash=deadbeef");
    }

    #[test]
    fn auth_query_moebooru_hashes_password_with_salt() {
        // konachan salt template, taken from builtin defaults.
        let salt = "So-I-Heard_You_Like_Mupkids.{}--";
        let salted = salt.replacen("{}", "hunter2", 1);
        let expected = {
            use sha1::{Digest, Sha1};
            hex::encode(Sha1::digest(salted.as_bytes()))
        };
        let mut s = site(BooruApiKind::Moebooru);
        s.login = Some("alice".into());
        s.password = Some("hunter2".into());
        s.password_salt = Some(salt.into());
        let q = auth_query(&s);
        assert!(q.starts_with("&login=alice&password_hash="));
        assert!(q.ends_with(&expected));
    }

    #[test]
    fn auth_query_moebooru_skips_when_login_missing() {
        let mut s = site(BooruApiKind::Moebooru);
        s.password_hash = Some("abc".into());
        assert_eq!(auth_query(&s), "");
    }

    #[test]
    fn auth_query_moebooru_skips_when_password_set_but_no_salt() {
        let mut s = site(BooruApiKind::Moebooru);
        s.login = Some("alice".into());
        s.password = Some("hunter2".into());
        // No salt → can't hash → anonymous.
        assert_eq!(auth_query(&s), "");
    }
}
