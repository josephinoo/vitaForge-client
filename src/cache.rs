//! On-disk catalog cache gated by /apps/version (like the Vita app).
use crate::api::{self, App, Version};
use std::{fs, path::PathBuf};

pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    base.join("vitaforge-client")
}

/// Cached snapshot if version unchanged; otherwise fetch and store. Falls back to stale cache offline.
pub fn catalog() -> Vec<App> {
    let d = dir();
    let cached = || -> Option<(Version, Vec<App>)> {
        Some((serde_json::from_slice(&fs::read(d.join("version-v3.json")).ok()?).ok()?,
              serde_json::from_slice(&fs::read(d.join("catalog-v3.json")).ok()?).ok()?))
    };
    let old = cached();
    match api::version() {
        Ok(v) if old.as_ref().is_some_and(|(o, _)| *o == v) => old.unwrap().1,
        Ok(v) => match api::snapshot() {
            Ok(apps) => {
                let _ = fs::create_dir_all(&d);
                let _ = fs::write(d.join("catalog-v3.json"), serde_json::to_vec(&apps).unwrap_or_default());
                let _ = fs::write(d.join("version-v3.json"), serde_json::to_vec(&v).unwrap_or_default());
                apps
            }
            Err(_) => old.map(|o| o.1).unwrap_or_default(),
        },
        Err(_) => old.map(|o| o.1).unwrap_or_default(),
    }
}
