//! Put a catalog app onto a mounted Vita volume.
//! VPK -> extracted to ux0/data/vitaforge/pending/<TITLE_ID>/ + manifest JSON (VitaForge promotes it on disconnect/boot);
//! game data zip -> extracted in place.
use crate::{api::{App, UA}, download::{download, Progress}};
use serde::Serialize;
use std::{fs, io, path::{Component, Path, PathBuf}, sync::Arc};

#[derive(Serialize)]
struct PendingManifest<'a> {
    title_id: &'a str,
    title: &'a str,
    version: &'a str,
    hash: &'a str,
    staged_dir: String,
}

/// `ux0:data/x`, `ux0:/data/x` or `data/x` -> `<root>/data/x`; rejects escapes.
pub fn resolve(root: &Path, p: &str) -> Option<PathBuf> {
    let rel = p.split_once(':').map_or(p, |(_, r)| r).trim_start_matches('/');
    let rel = Path::new(rel);
    rel.components().all(|c| matches!(c, Component::Normal(_))).then(|| root.join(rel))
}

pub fn extract_zip(zip: &Path, dest: &Path) -> io::Result<()> {
    let mut z = zip::ZipArchive::new(fs::File::open(zip)?).map_err(io::Error::other)?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(io::Error::other)?;
        let Some(rel) = f.enclosed_name() else { continue }; // skips ../ entries
        let out = dest.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            fs::create_dir_all(out.parent().unwrap())?;
            io::copy(&mut f, &mut fs::File::create(out)?)?;
        }
    }
    Ok(())
}

/// Extract a VPK into `ux0/data/vitaforge/pending/<title_id>/` and write its manifest for VitaForge.
pub fn stage_vpk(vpk: &Path, root: &Path, title_id: &str, title: &str, version: &str) -> io::Result<()> {
    let pending = root.join("data/vitaforge/pending");
    let staged = pending.join(title_id);
    if staged.exists() {
        fs::remove_dir_all(&staged)?;
    }
    fs::create_dir_all(&staged)?;
    extract_zip(vpk, &staged)?;
    let manifest = PendingManifest {
        title_id,
        title,
        version,
        hash: "",
        staged_dir: format!("ux0:data/vitaforge/pending/{title_id}"),
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?;
    fs::write(pending.join(format!("{title_id}.json")), json)
}

pub fn install(app: &App, root: &Path, p: &Arc<Progress>) -> io::Result<String> {
    let tmp = std::env::temp_dir().join("vitaforge-client");
    let mut done: Vec<String> = vec![];
    if let Some(url) = app.download_url.as_deref().filter(|u| u.starts_with("http")) {
        let title_id = app
            .title_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(str::trim)
            .unwrap_or("APP")
            .to_uppercase();
        let tmp_vpk = tmp.join(format!("{title_id}-{}.vpk", app.id));
        download(url, &tmp_vpk, UA, p)?;
        stage_vpk(&tmp_vpk, root, &title_id, &app.name, app.version.as_deref().unwrap_or(""))?;
        let _ = fs::remove_file(&tmp_vpk);
        done.push("listo para instalar en VitaForge (desconectá el USB)".into());
    }
    if let Some(url) = app.data_url.as_deref().filter(|u| u.starts_with("http")) {
        let target = resolve(root, app.data_extract_path.as_deref().unwrap_or("data"))
            .ok_or_else(|| io::Error::other("data_extract_path inválido"))?;
        let z = tmp.join(format!("data-{}.zip", app.id));
        download(url, &z, UA, p)?;
        extract_zip(&z, &target)?;
        fs::remove_file(z)?;
        done.push("game data extraído".into());
    }
    if done.is_empty() { Err(io::Error::other("sin download_url/data_url")) } else { Ok(done.join(" · ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resolve_blocks_escape() {
        let r = Path::new("/v");
        assert_eq!(resolve(r, "ux0:/data/x").unwrap(), Path::new("/v/data/x"));
        assert_eq!(resolve(r, "ux0:data").unwrap(), Path::new("/v/data"));
        assert!(resolve(r, "ux0:/../etc").is_none());
    }

    #[test]
    fn extracts_zip() {
        let d = std::env::temp_dir().join(format!("vf-zip-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let zp = d.join("a.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&zp).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("sub/f.txt", o).unwrap();
        w.write_all(b"hi").unwrap();
        w.start_file("../evil.txt", o).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
        extract_zip(&zp, &d.join("out")).unwrap();
        assert_eq!(fs::read(d.join("out/sub/f.txt")).unwrap(), b"hi");
        assert!(!d.join("evil.txt").exists());
        stage_vpk(&zp, &d, "ABCD12345", "Test App", "1.2.0").unwrap();
        assert_eq!(fs::read(d.join("data/vitaforge/pending/ABCD12345/sub/f.txt")).unwrap(), b"hi");
        let manifest = fs::read_to_string(d.join("data/vitaforge/pending/ABCD12345.json")).unwrap();
        assert!(manifest.contains("\"title_id\": \"ABCD12345\""));
        assert!(manifest.contains("ux0:data/vitaforge/pending/ABCD12345"));
        fs::remove_dir_all(d).unwrap();
    }
}
