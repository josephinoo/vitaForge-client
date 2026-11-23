//! Put a catalog app onto a mounted Vita volume.
//! VPK -> ux0/data/vitaforge/vpk/ (user installs it on the Vita); game data zip -> extracted in place.
use crate::{api::App, download::{download, Progress}, api::UA};
use std::{fs, io, path::{Component, Path, PathBuf}, sync::Arc};

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

pub fn install(app: &App, root: &Path, p: &Arc<Progress>) -> io::Result<String> {
    let tmp = std::env::temp_dir().join("vitaforge-client");
    let mut done = vec![];
    if let Some(url) = app.download_url.as_deref().filter(|u| u.starts_with("http")) {
        let name = format!("{}-{}.vpk", app.title_id.as_deref().unwrap_or("app"), app.id);
        let dest = root.join("data/vitaforge/vpk").join(&name);
        download(url, &tmp.join(&name), UA, p)?;
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(tmp.join(&name), &dest)?; // copy (not rename): tmp may be another filesystem
        fs::remove_file(tmp.join(&name))?;
        done.push(format!("VPK en data/vitaforge/vpk/{name} (instálalo desde la Vita)"));
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
        fs::remove_dir_all(d).unwrap();
    }
}
