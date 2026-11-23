//! Parallel, ranged HTTP download (same idea as the Vita app: probe Range, N parts, join).
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        Arc,
    },
    thread,
};

pub const PARTS: u64 = 4;

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

fn agent(ua: &str) -> ureq::Agent {
    ureq::AgentBuilder::new().user_agent(ua).build()
}

/// Returns total size if the server honours Range, else None.
fn probe(a: &ureq::Agent, url: &str) -> Option<u64> {
    let r = a.get(url).set("Range", "bytes=0-0").call().ok()?;
    if r.status() != 206 {
        return None;
    }
    r.header("Content-Range")?.rsplit('/').next()?.parse().ok()
}

fn fetch(a: &ureq::Agent, url: &str, range: Option<(u64, u64)>, out: &mut File, p: &Progress) -> io::Result<()> {
    let mut req = a.get(url);
    if let Some((s, e)) = range {
        req = req.set("Range", &format!("bytes={s}-{e}"));
    }
    let mut r = req.call().map_err(io::Error::other)?.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(&buf[..n])?;
        p.done.fetch_add(n as u64, Relaxed);
    }
}

pub fn download(url: &str, dest: &Path, ua: &str, p: &Arc<Progress>) -> io::Result<()> {
    let a = agent(ua);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    p.done.store(0, Relaxed);
    match probe(&a, url).filter(|&t| t >= PARTS) {
        Some(total) => {
            p.total.store(total, Relaxed);
            let size = total.div_ceil(PARTS);
            let handles: Vec<_> = (0..PARTS)
                .map(|i| {
                    let (a, url, p) = (a.clone(), url.to_string(), p.clone());
                    let part = dest.with_extension(format!("part{i}"));
                    let (s, e) = (i * size, ((i + 1) * size).min(total) - 1);
                    thread::spawn(move || -> io::Result<_> {
                        fetch(&a, &url, Some((s, e)), &mut File::create(&part)?, &p)?;
                        Ok(part)
                    })
                })
                .collect();
            let parts: Vec<_> = handles
                .into_iter()
                .map(|h| h.join().map_err(|_| io::Error::other("thread panicked"))?)
                .collect::<io::Result<_>>()?;
            let mut out = File::create(dest)?;
            for part in &parts {
                io::copy(&mut File::open(part)?, &mut out)?;
                fs::remove_file(part)?;
            }
        }
        None => {
            p.total.store(0, Relaxed);
            fetch(&a, url, None, &mut File::create(dest)?, p)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve(body: Vec<u8>, ranges: bool) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/f", l.local_addr().unwrap());
        thread::spawn(move || {
            for s in l.incoming().flatten() {
                let body = body.clone();
                thread::spawn(move || {
                    let mut s = s;
                    let mut buf = [0u8; 2048];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    let range = req.lines().find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.split_once('-'))
                        .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)));
                    let (head, data) = match range.filter(|_| ranges) {
                        Some((a, b)) => (
                            format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len(), b - a + 1),
                            body[a..=b].to_vec(),
                        ),
                        None => (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()), body),
                    };
                    let _ = s.write_all(head.as_bytes());
                    let _ = s.write_all(&data);
                });
            }
        });
        url
    }

    #[test]
    fn parallel_and_fallback_match() {
        let body: Vec<u8> = (0..1_000_003u32).map(|i| (i % 251) as u8).collect();
        for ranges in [true, false] {
            let url = serve(body.clone(), ranges);
            let dest = std::env::temp_dir().join(format!("vf-dl-{}-{ranges}", std::process::id()));
            let p = Arc::new(Progress::default());
            download(&url, &dest, "test", &p).unwrap();
            assert_eq!(fs::read(&dest).unwrap(), body, "ranges={ranges}");
            assert_eq!(p.done.load(Relaxed), body.len() as u64);
            fs::remove_file(dest).unwrap();
        }
    }
}
