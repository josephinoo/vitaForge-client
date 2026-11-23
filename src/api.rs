//! VitaForge catalog client (public API). Same cache gate as the Vita app:
//! /apps/version, then /apps/snapshot only if it changed.
use serde::{Deserialize, Serialize};

/// API base URL from the VITAFORGE_API env var (set in .env, never committed).
pub fn base() -> Result<&'static str, String> {
    static B: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    B.get_or_init(|| std::env::var("VITAFORGE_API").ok().map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| v.starts_with("http")))
        .as_deref()
        .ok_or_else(|| "Falta VITAFORGE_API en .env (ver .env.example)".to_string())
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
    fn app_tolerates_missing_fields() {
        let a: App = serde_json::from_str(r#"{"id":1,"name":"X","type":"psv_game"}"#).unwrap();
        assert_eq!(a.kind.as_deref(), Some("psv_game"));
        assert!(a.download_url.is_none());
    }
}
