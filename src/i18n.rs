//! Fluent-based translations (same approach as the Vita app): one .ftl per language, embedded at build time.
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use std::{cell::RefCell, fs, sync::atomic::{AtomicU8, Ordering::Relaxed}};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lang {
    Es,
    En,
    Fr,
    It,
    Pt,
}

const FTL: [&str; 5] = [
    include_str!("i18n/es.ftl"),
    include_str!("i18n/en.ftl"),
    include_str!("i18n/fr.ftl"),
    include_str!("i18n/it.ftl"),
    include_str!("i18n/pt.ftl"),
];

static CURRENT: AtomicU8 = AtomicU8::new(1);

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::Es, Lang::En, Lang::Fr, Lang::It, Lang::Pt];

    pub fn code(self) -> &'static str {
        ["es", "en", "fr", "it", "pt"][self as usize]
    }
    pub fn label(self) -> &'static str {
        ["Español", "English", "Français", "Italiano", "Português"][self as usize]
    }
    fn from_code(c: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| c.to_lowercase().starts_with(l.code()))
    }
}

pub fn current() -> Lang {
    Lang::ALL[CURRENT.load(Relaxed) as usize]
}

fn saved() -> std::path::PathBuf {
    crate::cache::dir().join("lang")
}

pub fn set(l: Lang) {
    CURRENT.store(l as u8, Relaxed);
    let _ = fs::create_dir_all(crate::cache::dir());
    let _ = fs::write(saved(), l.code());
}

/// Next language in the list (for the header switcher).
pub fn cycle() {
    set(Lang::ALL[(current() as usize + 1) % Lang::ALL.len()]);
}

/// Saved choice, else system locale, else English.
pub fn init() {
    let sys = || {
        let env = std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX");
        #[cfg(target_os = "macos")]
        let env = env.or_else(|| {
            let o = std::process::Command::new("defaults").args(["read", "-g", "AppleLocale"]).output().ok()?;
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        });
        env.as_deref().and_then(Lang::from_code)
    };
    let l = fs::read_to_string(saved()).ok().and_then(|c| Lang::from_code(&c)).or_else(sys).unwrap_or(Lang::En);
    CURRENT.store(l as u8, Relaxed);
}

fn bundle(l: Lang) -> FluentBundle<FluentResource> {
    let id = l.code().parse().expect("langid");
    let mut b = FluentBundle::new(vec![id]);
    b.set_use_isolating(false); // no bidi marks around arguments
    b.add_resource(FluentResource::try_new(FTL[l as usize].to_string()).expect("ftl")).expect("ftl ids");
    b
}

fn lookup(l: Lang, id: &str, args: Option<&FluentArgs>) -> Option<String> {
    thread_local! {
        static B: RefCell<[Option<FluentBundle<FluentResource>>; 5]> = RefCell::new(Default::default());
    }
    B.with(|b| {
        let mut b = b.borrow_mut();
        let b = b[l as usize].get_or_insert_with(|| bundle(l));
        let p = b.get_message(id)?.value()?;
        Some(b.format_pattern(p, args, &mut vec![]).into_owned())
    })
}

/// Translate `id`; falls back to English, then to the id itself.
pub fn t(id: &str) -> String {
    lookup(current(), id, None).or_else(|| lookup(Lang::En, id, None)).unwrap_or_else(|| id.to_string())
}

pub fn t1(id: &str, args: &[(&str, String)]) -> String {
    let mut a = FluentArgs::new();
    for (k, v) in args {
        a.set(*k, v.clone());
    }
    lookup(current(), id, Some(&a)).or_else(|| lookup(Lang::En, id, Some(&a))).unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_language_has_every_key() {
        let keys: Vec<&str> = FTL[1].lines().filter_map(|l| l.split_once(" = ").map(|x| x.0)).collect();
        for l in Lang::ALL {
            for k in &keys {
                assert!(lookup(l, k, Some(&FluentArgs::new())).is_some(), "{} missing {k}", l.code());
            }
        }
    }
}
