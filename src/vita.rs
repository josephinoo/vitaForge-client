//! Detect a USB-mounted Vita (VitaShell USB mode) and read its VitaForge state.
use crate::sfo;
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize, Clone)]
pub struct Auth {
    #[serde(default)]
    pub user_id: i64,
    pub username: String,
    pub token: String, // secret: never log
    #[serde(default)]
    pub role: String,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Auth({}, {}, <token>)", self.username, self.role)
    }
}

#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub title_id: String,
    pub title: String,
    pub version: Option<String>,
    pub state: Option<String>, // "installed" | "outdated" from vitaForge cache
}

#[derive(Debug)]
pub struct Vita {
    pub root: PathBuf, // dir that contains app/ and data/
    pub auth: Option<Auth>,
    pub client_id: Option<String>,
    pub apps: Vec<InstalledApp>,
}

/// Returns the ux0 root if `p` looks like a Vita volume (root of ux0:, or containing ux0/).
pub fn ux0_root(p: &Path) -> Option<PathBuf> {
    [p.to_path_buf(), p.join("ux0")]
        .into_iter()
        .find(|c| c.join("app").is_dir() && c.join("data").is_dir())
}

/// Mount parents to watch per OS.
pub fn mount_parents() -> Vec<PathBuf> {
    ["/Volumes", "/media", "/run/media", "/mnt"].map(PathBuf::from).into()
}

pub fn find() -> Option<Vita> {
    let mut cands = Vec::new();
    for parent in mount_parents() {
        for e in fs::read_dir(&parent).into_iter().flatten().flatten() {
            cands.push(e.path());
            // /run/media/<user>/<vol>
            for e2 in fs::read_dir(e.path()).into_iter().flatten().flatten() {
                cands.push(e2.path());
            }
        }
    }
    #[cfg(windows)]
    cands.extend((b'D'..=b'Z').map(|c| PathBuf::from(format!("{}:\\", c as char))));
    cands.iter().find_map(|p| load(p))
}

pub fn load(p: &Path) -> Option<Vita> {
    let root = ux0_root(p)?;
    let vf = root.join("data/vitaforge");
    let auth = fs::read_to_string(vf.join("auth.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let client_id = fs::read_to_string(vf.join("client_id"))
        .ok()
        .map(|s| s.trim().to_string());
    let cache = parse_cache(&fs::read_to_string(vf.join("installed_cache.json")).unwrap_or_default());
    let mut apps: Vec<InstalledApp> = fs::read_dir(root.join("app"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let title_id = e.file_name().to_string_lossy().into_owned();
            let sfo = fs::read(e.path().join("sce_sys/param.sfo")).ok()?;
            let m = sfo::parse(&sfo)?;
            Some(InstalledApp {
                title: sfo::get_str(&m, "TITLE").unwrap_or_else(|| title_id.clone()),
                version: fs::read_to_string(vf.join(format!("hashes/{title_id}.ver")))
                    .ok()
                    .map(|v| v.trim().to_string())
                    .or_else(|| sfo::get_str(&m, "APP_VER")),
                state: cache.get(&title_id).cloned(),
                title_id,
            })
        })
        .collect();
    apps.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Some(Vita { root, auth, client_id, apps })
}

/// installed_cache.json is NOT json: one `TITLEID=installed|outdated` per line.
pub fn parse_cache(s: &str) -> HashMap<String, String> {
    s.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_uppercase(), v.trim().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_reads_fake_vita() {
        let d = std::env::temp_dir().join(format!("vf-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ux0 = d.join("ux0");
        fs::create_dir_all(ux0.join("app/ABCD12345/sce_sys")).unwrap();
        fs::create_dir_all(ux0.join("data/vitaforge")).unwrap();
        fs::write(
            ux0.join("app/ABCD12345/sce_sys/param.sfo"),
            sfo::build(&[("TITLE", "My Game"), ("APP_VER", "01.00")]),
        )
        .unwrap();
        fs::write(
            ux0.join("data/vitaforge/auth.json"),
            r#"{"user_id":1,"username":"joe","token":"sekret","role":"admin"}"#,
        )
        .unwrap();
        fs::write(ux0.join("data/vitaforge/client_id"), "abc123\n").unwrap();
        fs::write(ux0.join("data/vitaforge/installed_cache.json"), "ABCD12345=outdated\n").unwrap();

        let v = load(&d).expect("volume with ux0/ subdir");
        assert_eq!(v.auth.as_ref().unwrap().username, "joe");
        assert!(!format!("{:?}", v.auth).contains("sekret"));
        assert_eq!(v.client_id.as_deref(), Some("abc123"));
        assert_eq!(v.apps[0].title, "My Game");
        assert_eq!(v.apps[0].version.as_deref(), Some("01.00"));
        assert_eq!(v.apps[0].state.as_deref(), Some("outdated"));
        assert!(load(&std::env::temp_dir().join("nope-xyz")).is_none());
        fs::remove_dir_all(&d).unwrap();
    }
}
