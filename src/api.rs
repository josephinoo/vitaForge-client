//! VitaForge catalog client (public API). Same cache gate as the Vita app:
//! /apps/version, then /apps/snapshot only if it changed.
use serde::{Deserialize, Serialize};

/// API base URL from the VITAFORGE_API env var (set in .env, never committed).
pub fn base() -> Result<&'static str, String> {
    static B: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    // runtime env/.env first (dev), else the value baked in at build time by CI (release builds)
    B.get_or_init(|| std::env::var("VITAFORGE_API").ok().or_else(|| option_env!("VITAFORGE_API").map(String::from)).map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| v.starts_with("http")))
        .as_deref()
        .ok_or_else(|| crate::i18n::t("missing-config"))
}

/// Server origin (scheme + host) derived from the base URL, for relative image paths.
pub fn origin() -> Option<&'static str> {
    let b = base().ok()?;
    Some(b.split_once("/api/").map_or(b, |(o, _)| o))
}
pub const UA: &str = concat!("VitaForgeClient/", env!("CARGO_PKG_VERSION"));

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct App {
    pub id: i64,
    #[serde(default)]
    pub title_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub data_url: Option<String>,
    #[serde(default)]
    pub data_extract_path: Option<String>,
    #[serde(default)]
    pub average_rating: Option<f64>,
    #[serde(default)]
    pub install_count: Option<i64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub long_description: Option<String>,
    #[serde(default)]
    pub requirements: Option<String>,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub screenshot_urls: Vec<String>,
    /// Package / extra-data size in bytes (the server sends them as strings).
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub data_size: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub ratings_count: Option<i64>,
    #[serde(default)]
    pub likes_count: Option<i64>,
}

/// Sidebar categories, same set and order as the Vita app.
pub const CATEGORIES: [&str; 7] = ["Emulator", "Original", "PS Vita Game", "Plugin", "Port", "Utility", "Theme"];

/// Normalised category (mirrors the Vita app's `Category::from_api`); None = "Other"/"Tool", only under Todo.
pub fn category(a: &App) -> Option<&'static str> {
    let kind = a.kind.as_deref().unwrap_or("").trim().to_lowercase();
    match kind.as_str() {
        "plugin" => return Some("Plugin"),
        "theme" => return Some("Theme"),
        _ => {}
    }
    match a.category.as_deref().unwrap_or("").trim().to_lowercase().as_str() {
        "emulator" | "emulators" => Some("Emulator"),
        "original" => Some("Original"),
        "game" | "games" | "ps vita game" => Some("PS Vita Game"),
        "plugin" | "plugins" => Some("Plugin"),
        "port" | "ports" => Some("Port"),
        "utility" | "utilities" => Some("Utility"),
        "tool" | "tools" => None,
        _ if kind == "psv_game" => Some("PS Vita Game"),
        _ => None,
    }
}

/// The Vita app drops entries without a usable download URL.
pub fn installable(a: &App) -> bool {
    a.download_url.as_deref().is_some_and(|u| !u.is_empty() && !u.eq_ignore_ascii_case("missing"))
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Version {
    pub version: i64,
    pub total_apps: i64,
}

#[derive(Deserialize)]
struct Snapshot {
    data: Vec<App>,
}

fn get(path: &str) -> Result<ureq::Request, String> {
    Ok(ureq::get(&format!("{}{path}", base()?)).set("User-Agent", UA))
}

pub fn version() -> Result<Version, String> {
    get("/apps/version")?.call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())
}

pub fn snapshot() -> Result<Vec<App>, String> {
    let r = get("/apps/snapshot")?.call().map_err(|e| e.to_string())?;
    Ok(r.into_json::<Snapshot>().map_err(|e| e.to_string())?.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn category_normalises_like_the_vita_app() {
        let c = |cat: &str, kind: &str| category(&App { category: Some(cat.into()), kind: Some(kind.into()), ..Default::default() });
        assert_eq!(c("Games", "app"), Some("PS Vita Game"));
        assert_eq!(c("port", "app"), Some("Port"));
        assert_eq!(c("Game Port", "app"), None);
        assert_eq!(c("Original", "plugin"), Some("Plugin"));
        assert_eq!(c("", "psv_game"), Some("PS Vita Game"));
    }

    #[test]
    fn app_tolerates_missing_fields() {
        let a: App = serde_json::from_str(r#"{"id":1,"name":"X","type":"psv_game"}"#).unwrap();
        assert_eq!(a.kind.as_deref(), Some("psv_game"));
        assert!(a.download_url.is_none());
    }
}
