//! Login against the VitaForge server; the session is kept in the cache dir.
use crate::{api, cache};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct Session {
    pub username: String,
    #[serde(default)]
    pub role: String,
    pub token: String, // secret: never log
}

fn path() -> PathBuf {
    cache::dir().join("session.json")
}

pub fn load() -> Option<Session> {
    serde_json::from_slice(&fs::read(path()).ok()?).ok()
}

pub fn logout() {
    let _ = fs::remove_file(path());
}

fn save(s: &Session) {
    let _ = fs::create_dir_all(cache::dir());
    if fs::write(path(), serde_json::to_vec(s).unwrap_or_default()).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path(), fs::Permissions::from_mode(0o600));
        }
    }
}

#[derive(Deserialize)]
struct Reply {
    username: String,
    #[serde(default)]
    role: String,
    token: Option<String>,
}

/// POST {origin}/login; the token comes in the JSON body or the `vf_token` cookie.
pub fn login(user: &str, pass: &str) -> Result<Session, String> {
    let url = format!("{}/login", api::origin().ok_or_else(|| crate::i18n::t("missing-config"))?);
    let resp = match ureq::post(&url).set("User-Agent", api::UA).send_json(serde_json::json!({ "username": user, "password": pass })) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let msg = r.into_json::<serde_json::Value>().ok().and_then(|v| v["error"].as_str().map(String::from));
            return Err(msg.unwrap_or_else(|| format!("HTTP {code}")));
        }
        Err(e) => return Err(e.to_string()),
    };
    let cookie = resp.all("set-cookie").into_iter().find_map(|c| {
        let v = c.split_once("vf_token=")?.1.split(';').next()?.trim();
        (!v.is_empty()).then(|| v.to_string())
    });
    let r: Reply = resp.into_json().map_err(|e| e.to_string())?;
    let s = Session { username: r.username, role: r.role, token: r.token.or(cookie).ok_or("No token in response")? };
    save(&s);
    Ok(s)
}
