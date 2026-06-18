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
