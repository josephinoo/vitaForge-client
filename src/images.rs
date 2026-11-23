//! Cover/icon cache: at most WORKERS concurrent fetches, each URL requested once
//! (failures are remembered, 429/5xx retried with backoff), so repainting never floods a server.
use crate::api::{base, origin, App, UA};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

const WORKERS: usize = 4;

struct Job {
    url: String,
    dest: PathBuf,
}

struct Pool {
    tx: mpsc::Sender<Job>,
    seen: Mutex<HashSet<PathBuf>>, // queued, in flight, done or failed: never requested twice
}

fn pool() -> &'static Pool {
    static P: OnceLock<Pool> = OnceLock::new();
    P.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..WORKERS {
            let rx = rx.clone();
            thread::spawn(move || loop {
                let Ok(job) = rx.lock().unwrap().recv() else { return };
                fetch(&job);
            });
        }
        Pool { tx, seen: Mutex::default() }
    })
}

fn fetch(j: &Job) {
    for attempt in 0..3u64 {
        match ureq::get(&j.url).set("User-Agent", UA).call() {
            Ok(r) => {
                let mut b = Vec::new();
                if r.into_reader().take(8 << 20).read_to_end(&mut b).is_ok() && !b.is_empty() {
                    let _ = fs::create_dir_all(j.dest.parent().unwrap());
                    let tmp = j.dest.with_extension("tmp");
                    if fs::write(&tmp, b).is_ok() {
                        let _ = fs::rename(tmp, &j.dest); // atomic: UI never reads a half file
                    }
                }
                return;
            }
            Err(ureq::Error::Status(c, _)) if c == 429 || c >= 500 => {
                thread::sleep(Duration::from_secs(2 << attempt));
            }
            Err(_) => return,
        }
    }
}

fn source(a: &App) -> Option<(String, &'static str)> {
    let (origin, base) = (origin()?, base().ok()?);
    if let Some(c) = a.cover_url.as_deref().filter(|c| !c.is_empty()) {
        let u = if c.starts_with("http") { c.to_string() } else { format!("{origin}{c}") };
        return Some((u.replace("size=medium", "size=thumb"), "jpg"));
    }
    if let Some(i) = a.icon_url.as_deref().filter(|i| !i.is_empty()) {
        let u = if i.starts_with("http") { i.to_string() } else { format!("{origin}{i}") };
        return Some((u, "png"));
    }
    a.title_id.as_deref().filter(|_| a.kind.as_deref() == Some("psv_game"))
        .map(|t| (format!("{base}/images/cover/{t}?size=thumb&format=jpeg"), "jpg"))
}

/// Cached file if present; otherwise queue a fetch (once) and return None.
pub fn path(a: &App) -> Option<PathBuf> {
    let (url, ext) = source(a)?;
    let dest = std::env::temp_dir().join("vitaforge-client-img").join(format!("{}.{ext}", a.id));
    if dest.exists() {
        return Some(dest);
    }
    let p = pool();
    if p.seen.lock().unwrap().insert(dest.clone()) {
        let _ = p.tx.send(Job { url, dest });
    }
    None
}
