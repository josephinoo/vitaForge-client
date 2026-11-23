#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use std::{
    collections::HashSet,
    sync::{atomic::Ordering::Relaxed, Arc, Mutex},
    time::{Duration, Instant},
};
use vitaforge_client::{account, api, cache, download::Progress, i18n::{self, t, t1}, images, install, vita};

const BG: u32 = 0x070c18;
const HEADER: u32 = 0x0c1426;
const CARD: u32 = 0x131d33;
const HOVER: u32 = 0x1c2a47;
const LINE: u32 = 0x22314f;
const TEXT: u32 = 0xf4f7fb;
const DIM: u32 = 0x8298bc;
const ACCENT: u32 = 0x3890ff;
const OK: u32 = 0x30d158;
const WARN: u32 = 0xffd60a;
const ERR: u32 = 0xff5a52;
const RAIL: f32 = 76.;

/// Small rounded label: `color` is used for the text and (translucent) for the background.
fn chip(text: impl Into<SharedString>, color: u32) -> Div {
    div().px_2().py(px(2.)).rounded_full().text_xs().text_color(rgb(color)).bg(rgba((color << 8) | 0x26)).child(text.into())
}

fn progress_bar(frac: f32, color: u32) -> Div {
    div().h(px(6.)).w_full().rounded_full().bg(rgb(BG)).child(div().h_full().w(relative(frac.clamp(0.0, 1.0))).rounded_full().bg(rgb(color)))
}

/// Lucide icon from the bundled catalog (`icons/<name>.svg`), tinted with `color`.
fn icon(name: &str, size: f32, color: u32) -> Svg {
    svg().path(format!("icons/{name}.svg")).size(px(size)).flex_none().text_color(rgb(color))
}

fn empty(name: &'static str, text: String) -> AnyElement {
    div().size_full().flex().flex_col().items_center().justify_center().gap_2()
        .child(icon(name, 44., DIM))
        .child(div().text_color(rgb(DIM)).child(text))
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Catalog,
    Library,
    Transfers,
    Settings,
}

struct Transfer {
    name: String,
    progress: Arc<Progress>,
    result: Arc<Mutex<Option<String>>>,
}

struct Main {
    loading: bool,
    started: Instant,
    logo: Arc<Image>,
    bg: Arc<Image>,
    apps: Vec<api::App>,
    filtered: Vec<usize>,
    popular: Vec<usize>,
    rated: Vec<usize>,
    cats: Vec<(String, usize)>,
    cat: Option<String>,
    query: Entity<InputState>,
    tab: Tab,
    selected: Option<usize>,
    vita: Option<vita::Vita>,
    transfers: Vec<Transfer>,
    status: SharedString,
    session: Option<account::Session>,
    user_in: Entity<InputState>,
    pass_in: Entity<InputState>,
    login_err: SharedString,
    logging: bool,
    _sub: Subscription,
}

fn thousands(n: i64) -> String {
    let d = n.to_string();
    let mut o = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 && c != '-' { o.push('.'); }
        o.push(c);
    }
    o
}

fn size(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

impl Main {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(t("search")));
        let user_in = cx.new(|cx| InputState::new(window, cx).placeholder(t("username")));
        let pass_in = cx.new(|cx| InputState::new(window, cx).placeholder(t("password")).masked(true));
        let _sub = cx.subscribe(&query, |this: &mut Self, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        // load the catalog off the UI thread so the splash can animate
        cx.spawn(async move |this, cx| {
            let apps = cx.background_executor().spawn(async { cache::catalog() }).await;
            this.update(cx, |t, cx| t.set_catalog(apps, cx)).ok();
        })
        .detach();
        // repaint: fast while the splash spins, slow afterwards (progress bars, icons)
        cx.spawn(async move |this, cx| loop {
            let Ok(loading) = this.update(cx, |t, cx| {
                cx.notify();
                t.loading
            }) else {
                break;
            };
            cx.background_executor().timer(Duration::from_millis(if loading { 50 } else { 500 })).await;
        })
        .detach();
        let png = |b: &[u8]| Arc::new(Image::from_bytes(ImageFormat::Png, b.to_vec()));
        Self {
            loading: true,
            started: Instant::now(),
            logo: png(include_bytes!("../../assets/logo.png")),
            bg: png(include_bytes!("../../assets/bg.png")),
            filtered: vec![],
            popular: vec![],
            rated: vec![],
            apps: vec![],
            cats: vec![],
            cat: None,
            query,
            tab: Tab::Catalog,
            selected: None,
            vita: vita::find(),
            transfers: vec![],
            status: "".into(),
            session: account::load(),
            user_in,
            pass_in,
            login_err: "".into(),
            logging: false,
            _sub,
        }
    }

    fn set_catalog(&mut self, mut apps: Vec<api::App>, cx: &mut Context<Self>) {
        // PS1/PSP entries are not offered (same as the Vita app; their art is behind a bot challenge)
        apps.retain(|a| api::installable(a) && !matches!(a.kind.as_deref(), Some("psx_game" | "psp_game" | "psp_app")));
        self.cats = api::CATEGORIES
            .iter()
            .map(|c| (c.to_string(), apps.iter().filter(|a| api::category(a) == Some(*c)).count()))
            .collect();
        self.status = match (apps.is_empty(), api::base()) {
            (_, Err(e)) => e.into(),
            (true, _) => t("no-connection").into(),
            _ => t("catalog-loaded").into(),
        };
        let top = |key: &dyn Fn(&api::App) -> i64| {
            let mut v: Vec<usize> = (0..apps.len()).collect();
            v.sort_by_key(|&i| std::cmp::Reverse(key(&apps[i])));
            v.truncate(20);
            v
        };
        self.popular = top(&|a| a.install_count.unwrap_or(0));
        self.rated = top(&|a| ((a.average_rating.unwrap_or(0.0) * 100.0) as i64) * (a.install_count.unwrap_or(0) > 20) as i64);
        self.apps = apps;
        self.loading = false;
        self.refilter(cx);
    }

    fn splash(&self) -> AnyElement {
        let t = self.started.elapsed().as_secs_f32();
        let dots = (0..8).map(|i| {
            let ang = t * 4.0 + i as f32 * std::f32::consts::TAU / 8.0;
            let alpha = 0.2 + 0.8 * (i as f32 / 8.0).powf(1.5);
            div()
                .absolute()
                .left(px(24. + ang.cos() * 11.))
                .top(px(24. + ang.sin() * 11.))
                .size(px(4.4))
                .rounded_full()
                .bg(rgb(ACCENT))
                .opacity(alpha)
        });
        div()
            .size_full()
            .relative()
            .child(img(self.bg.clone()).size_full().absolute().object_fit(ObjectFit::Cover))
            .child(
                div().size_full().absolute().flex().flex_col().items_center().justify_center().gap_2()
                    .child(
                        img(self.logo.clone()).size(px(168.)).rounded(px(28.)).with_animation(
                            "logo-in",
                            Animation::new(Duration::from_millis(600)),
                            |e, p| e.opacity(p).size(px(140. + 28. * p)),
                        ),
                    )
                    .child(div().text_3xl().text_color(rgb(ACCENT)).child("VitaForge"))
                    .child(div().text_sm().text_color(rgb(DIM)).child(concat!("v", env!("CARGO_PKG_VERSION"))))
                    .child(div().text_xs().text_color(rgb(0x4d6288)).child("by josephinoo"))
                    .child(div().text_color(rgb(DIM)).child("Cargando catálogo…"))
                    .child(div().size(px(48.)).relative().children(dots)),
            )
            .into_any_element()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let q = self.query.read(cx).value().to_lowercase();
        self.filtered = (0..self.apps.len())
            .filter(|&i| {
                let a = &self.apps[i];
                self.cat.as_deref().is_none_or(|c| api::category(a) == Some(c))
                    && (q.is_empty() || a.name.to_lowercase().contains(&q))
            })
            .collect();
        cx.notify();
    }

    fn start_install(&mut self, i: usize) {
        let Some(root) = self.vita.as_ref().map(|v| v.root.clone()) else {
            self.status = t("connect-vita-first").into();
            return;
        };
        let app = self.apps[i].clone();
        let tr = Transfer { name: app.name.clone(), progress: Arc::default(), result: Arc::default() };
        let (p, r) = (tr.progress.clone(), tr.result.clone());
        std::thread::spawn(move || {
            let msg = match install::install(&app, &root, &p) {
                Ok(m) => m,
                Err(e) => format!("{}: {e}", t("error")),
            };
            *r.lock().unwrap() = Some(msg);
        });
        self.transfers.push(tr);
        self.tab = Tab::Transfers;
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let (u, p) = (self.user_in.read(cx).value().trim().to_string(), self.pass_in.read(cx).value().to_string());
        if u.is_empty() || p.is_empty() {
            self.login_err = t("login-missing").into();
            return;
        }
        self.logging = true;
        self.login_err = "".into();
        cx.spawn(async move |this, cx| {
            let r = cx.background_executor().spawn(async move { account::login(&u, &p) }).await;
            this.update(cx, |t_, cx| {
                t_.logging = false;
                match r {
                    Ok(s) => t_.session = Some(s),
                    Err(e) => t_.login_err = format!("{}: {e}", t("login-failed")).into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let card = |title: String| {
            div().p_4().gap_3().flex().flex_col().rounded(px(16.)).bg(rgb(CARD)).border_1().border_color(rgb(LINE))
                .child(div().text_xs().text_color(rgb(DIM)).child(title.to_uppercase()))
        };
        let account = match &self.session {
            Some(s) => card(t("account"))
                .child(div().flex().items_center().gap_2().child(t1("signed-in-as", &[("user", s.username.clone())])).children((!s.role.is_empty()).then(|| chip(s.role.clone(), ACCENT))))
                .child(Button::new("sign-out").label(t("sign-out")).on_click(cx.listener(|this, _, _, cx| {
                    account::logout();
                    this.session = None;
                    cx.notify();
                }))),
            None => card(t("account"))
                .child(div().w(px(320.)).child(Input::new(&self.user_in)))
                .child(div().w(px(320.)).child(Input::new(&self.pass_in)))
                .child(Button::new("sign-in").label(t(if self.logging { "signing-in" } else { "sign-in" })).on_click(cx.listener(|this, _, _, cx| {
                    if !this.logging {
                        this.sign_in(cx);
                    }
                    cx.notify();
                })))
                .children((!self.login_err.is_empty()).then(|| div().text_sm().text_color(rgb(ERR)).child(self.login_err.clone()))),
        };
        let langs = card(t("language")).child(div().flex().flex_wrap().gap_2().children(i18n::Lang::ALL.into_iter().map(|l| {
            let on = i18n::current() == l;
            div().id(l.code()).px_4().py(px(6.)).rounded_full().text_sm().cursor_pointer()
                .bg(rgb(if on { ACCENT } else { HOVER }))
                .child(l.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    i18n::set(l);
                    this.status = "".into();
                    cx.notify();
                }))
        })));
        let device = card(t("device"))
            .child(div().text_sm().text_color(rgb(DIM)).child(match &self.vita {
                Some(v) => v.root.display().to_string(),
                None => t("vita-none"),
            }))
            .child(Button::new("rescan").label(t("find-vita")).on_click(cx.listener(|this, _, _, cx| {
                this.rescan();
                cx.notify();
            })));
        let about = card(t("about")).child(div().text_sm().text_color(rgb(DIM)).child(format!("VitaForge Client v{}", env!("CARGO_PKG_VERSION"))));
        div().id("settings").size_full().px_6().pb_6().gap_4().flex().flex_col().overflow_y_scroll()
            .child(account).child(langs).child(device).child(about)
            .into_any_element()
    }

    fn rescan(&mut self) {
        self.vita = vita::find();
        self.status = t(if self.vita.is_some() { "vita-detected" } else { "vita-not-found" }).into();
    }

    fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let btn = |id: &'static str, name: &'static str, t: Tab, cx: &mut Context<Self>| {
            let on = self.tab == t;
            div().id(id).size(px(48.)).rounded(px(14.)).flex().items_center().justify_center().cursor_pointer()
                .bg(rgb(if on { CARD } else { HEADER }))
                .hover(|s| s.bg(rgb(HOVER)))
                .child(icon(name, 22., if on { ACCENT } else { DIM }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = t;
                    this.cat = None;
                    this.refilter(cx);
                }))
        };
        div().w(px(RAIL)).py_4().gap_3().flex().flex_col().items_center().bg(rgb(HEADER)).border_r_1().border_color(rgb(LINE))
            .child(img(self.logo.clone()).size(px(40.)).rounded(px(11.)))
            .child(div().h(px(12.)))
            .child(btn("r-home", "house", Tab::Catalog, cx))
            .child(btn("r-lib", "library", Tab::Library, cx))
            .child(btn("r-tr", "download", Tab::Transfers, cx))
            .child(div().flex_1())
            .child(btn("r-set", "settings", Tab::Settings, cx))
            .child(
                div().id("r-vita").size(px(48.)).rounded(px(14.)).flex().items_center().justify_center().cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .child(icon("gamepad-2", 22., if self.vita.is_some() { OK } else { DIM }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rescan();
                        cx.notify();
                    })),
            )
    }

    fn chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let one = |id: String, label: String, sel: bool, cat: Option<String>, cx: &mut Context<Self>| {
            div().id(SharedString::from(id)).px_4().py(px(6.)).rounded_full().text_sm().cursor_pointer()
                .bg(rgb(if sel { ACCENT } else { CARD }))
                .text_color(rgb(if sel { TEXT } else { DIM }))
                .hover(|s| s.bg(rgb(if sel { ACCENT } else { HOVER })))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cat = cat.clone();
                    this.refilter(cx);
                }))
        };
        div().id("chips").px_6().pb_3().gap_2().flex().overflow_x_scroll()
            .child(one("cat-all".into(), t("all"), self.cat.is_none(), None, cx))
            .children(self.cats.iter().map(|(c, n)| one(format!("cat-{c}"), format!("{c} · {n}"), self.cat.as_ref() == Some(c), Some(c.clone()), cx)))
    }

    fn poster(&self, i: usize, w: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let a = &self.apps[i];
        let h = w / Self::ratio(a);
        let cover = match images::path(a) {
            Some(p) => div().size_full().child(img(p).size_full().object_fit(ObjectFit::Cover)),
            None => div().size_full().bg(rgb(HOVER)).flex().items_center().justify_center()
                .child(div().text_3xl().text_color(rgb(ACCENT)).child(a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())),
        };
        div().id(("poster", i)).w(px(w)).flex_none().cursor_pointer().flex().flex_col().gap_1()
            .child(div().w(px(w)).h(px(h)).rounded(px(14.)).overflow_hidden().bg(rgb(CARD)).border_1().border_color(rgb(LINE)).shadow_md()
                .hover(|s| s.border_color(rgb(ACCENT))).child(cover))
            .child(div().px_1().text_sm().overflow_hidden().max_h(px(20.)).child(a.name.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(i);
                cx.notify();
            }))
    }

    fn row(&self, id: &'static str, title: String, list: &[usize], cx: &mut Context<Self>) -> Div {
        div().flex().flex_col().flex_none().gap_3()
            .child(div().flex().justify_between().items_end()
                .child(div().text_xl().child(title))
                .child(div().text_sm().text_color(rgb(DIM)).child(t1("apps", &[("count", list.len().to_string())]))))
            .child(div().id(id).flex().gap_4().overflow_x_scroll().pb_2().children(list.iter().map(|&i| self.poster(i, 150., cx))))
    }

    fn home(&self, cx: &mut Context<Self>) -> AnyElement {
        let hero = self.popular.iter().copied().find(|&i| self.apps[i].cover_url.is_some()).or(self.popular.first().copied());
        let Some(hi) = hero else { return empty("house", t("no-catalog")) };
        let a = &self.apps[hi];
        let mut banner = div().relative().flex_none().w_full().h(px(280.)).rounded(px(20.)).overflow_hidden().bg(rgb(CARD)).border_1().border_color(rgb(LINE));
        if let Some(p) = images::path(a) {
            banner = banner.child(img(p).absolute().top(px(-90.)).size_full().object_fit(ObjectFit::Cover));
        }
        let banner = banner
            .child(div().absolute().size_full().bg(rgba((BG << 8) | 0xb8)))
            .child(
                div().absolute().size_full().p_8().flex().flex_col().justify_end().gap_2()
                    .child(div().text_xs().text_color(rgb(ACCENT)).child(t("featured")))
                    .child(div().text_3xl().child(a.name.clone()))
                    .child(div().text_color(rgb(DIM)).child(format!("{} · {}", a.author.clone().unwrap_or_default(), t1("installs", &[("count", thousands(a.install_count.unwrap_or(0)))]))))
                    .child(div().flex().gap_2().pt_2()
                        .child(div().id("hero-install").px_5().py(px(10.)).rounded(px(12.)).bg(rgb(TEXT)).text_color(rgb(BG)).cursor_pointer().flex().items_center().gap_2().child(icon("play", 16., BG)).child(t("install"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_install(hi);
                                cx.notify();
                            })))
                        .child(div().id("hero-details").px_5().py(px(10.)).rounded(px(12.)).bg(rgb(HOVER)).cursor_pointer().child(t("details"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = Some(hi);
                                cx.notify();
                            })))),
            );
        div().id("home").size_full().px_6().pb_6().gap_6().flex().flex_col().overflow_y_scroll()
            .child(banner)
            .child(self.row("row-pop", t("most-installed"), &self.popular, cx))
            .child(self.row("row-rated", t("top-rated"), &self.rated, cx))
            .into_any_element()
    }

    /// Cover width/height: PS Vita art measured at 0.78; entries with only an icon are square.
    fn ratio(a: &api::App) -> f32 {
        if a.cover_url.is_some() { 0.78 } else { 1.0 }
    }

    fn grid(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        const GAP: f32 = 12.;
        const PAD: f32 = 24.;
        const SIDEBAR: f32 = RAIL;
        const TARGET: f32 = 170.;
        let avail = window.viewport_size().width.as_f32() - SIDEBAR - if self.selected.is_some() { 360. } else { 0. } - 2. * PAD;
        let cols = (((avail + GAP) / (TARGET + GAP)).floor() as usize).max(1);
        let slot_w = ((avail - GAP * (cols - 1) as f32) / cols as f32).max(60.);
        let slot_h = slot_w / 0.78;
        let rows = self.filtered.len().div_ceil(cols);
        uniform_list(
            "grid",
            rows,
            cx.processor(move |this, range: std::ops::Range<usize>, _w, cx| {
                let have: HashSet<&str> = this.vita.iter().flat_map(|v| v.apps.iter().map(|a| a.title_id.as_str())).collect();
                range
                    .map(|r| {
                        div().h(px(slot_h + 46.)).px(px(PAD)).gap(px(GAP)).flex().children(
                            this.filtered.iter().skip(r * cols).take(cols).map(|&i| {
                                let a = &this.apps[i];
                                let h = slot_w / Self::ratio(a);
                                let mut cover = div().relative().w(px(slot_w)).h(px(h)).rounded(px(14.)).bg(rgb(CARD)).overflow_hidden().shadow_md()
                                    .flex().items_center().justify_center();
                                match images::path(a) {
                                    Some(p) => cover = cover.child(img(p).size_full().object_fit(ObjectFit::Cover)),
                                    None => {
                                        let initial = a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                                        cover = cover.bg(rgb(HOVER)).child(div().text_3xl().text_color(rgb(ACCENT)).child(initial));
                                    }
                                }
                                if a.title_id.as_deref().is_some_and(|t| have.contains(t)) {
                                    cover = cover.child(div().absolute().top(px(8.)).left(px(8.)).child(chip(t("installed"), OK)).bg(rgba((BG << 8) | 0xcc)).rounded_full());
                                }
                                div()
                                    .id(("card", i))
                                    .w(px(slot_w))
                                    .h(px(slot_h + 40.))
                                    .cursor_pointer()
                                    .rounded(px(16.))
                                    .border_1()
                                    .border_color(rgba(0))
                                    .hover(|s| s.bg(rgb(CARD)).border_color(rgb(ACCENT)))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().w(px(slot_w)).h(px(slot_h)).flex().items_center().justify_center().child(cover))
                                    .child(div().px_1().text_sm().overflow_hidden().max_h(px(20.)).child(a.name.clone()))
                                    .child(div().px_1().text_xs().text_color(rgb(DIM)).overflow_hidden().max_h(px(16.)).child(a.author.clone().unwrap_or_default()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.selected = Some(i);
                                        cx.notify();
                                    }))
                            }),
                        )
                    })
                    .collect()
            }),
        )
        .flex_1()
    }

    fn detail(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let a = &self.apps[i];
        let bytes = |v: &Option<String>| v.as_deref().and_then(|s| s.parse::<u64>().ok()).filter(|&n| n > 0);
        let fmt = |n: u64| if n >= 1 << 30 { format!("{:.2} GB", n as f64 / (1u64 << 30) as f64) } else { size(n) };
        let total = bytes(&a.size).unwrap_or(0) + bytes(&a.data_size).unwrap_or(0);
        let stat = |ic: &'static str, label: String, v: String| {
            div().flex_1().p_2().rounded(px(10.)).bg(rgb(CARD)).flex().flex_col().gap_1()
                .child(div().flex().items_center().gap_1().text_xs().text_color(rgb(DIM)).child(icon(ic, 12., DIM)).child(label))
                .child(div().text_sm().child(v))
        };
        let section = |title: String, body: String| {
            div().flex().flex_col().gap_1().child(div().text_xs().text_color(rgb(DIM)).child(title)).child(div().text_sm().child(body))
        };
        let clip = |t: &str, n: usize| {
            let t = t.trim();
            if t.chars().count() > n { format!("{}…", t.chars().take(n).collect::<String>()) } else { t.to_string() }
        };
        let desc = a.long_description.as_deref().filter(|d| !d.trim().is_empty()).or(a.description.as_deref()).unwrap_or("");
        let mut d = div()
            .id("detail")
            .w(px(360.))
            .p_4()
            .gap_3()
            .flex()
            .flex_col()
            .flex_none()
            .overflow_y_scroll()
            .bg(rgb(HEADER))
            .border_l_1()
            .border_color(rgb(LINE))
            .children(images::path(a).map(|p| div().w(px(328.)).h(px(328. / Self::ratio(a))).flex_none().rounded(px(16.)).overflow_hidden().bg(rgb(CARD)).shadow_md().child(img(p).size_full().object_fit(ObjectFit::Contain))))
            .child(div().text_xl().child(a.name.clone()))
            .child(div().text_color(rgb(DIM)).child(a.author.clone().unwrap_or_default()))
            .child(div().flex().gap_2().flex_wrap()
                .child(chip(a.category.clone().unwrap_or_default(), ACCENT))
                .child(chip(format!("v{}", a.version.clone().unwrap_or_default().trim_start_matches(['v', 'V'])), DIM)))
            .child(div().flex().flex_wrap().gap_x_4().gap_y_1().text_sm()
                .child(div().flex().items_center().gap_1().text_color(rgb(WARN)).child(icon("star-fill", 14., WARN)).child(format!("{:.1} ({})", a.average_rating.unwrap_or(0.0), a.ratings_count.unwrap_or(0))))
                .child(div().flex().items_center().gap_1().text_color(rgb(DIM)).child(icon("download", 14., DIM)).child(thousands(a.install_count.unwrap_or(0))))
                .child(div().flex().items_center().gap_1().text_color(rgb(DIM)).child(icon("heart", 14., DIM)).child(thousands(a.likes_count.unwrap_or(0)))));
        if a.download_url.is_some() || a.data_url.is_some() {
            d = d.child(Button::new("install").label(t("install-on-vita")).on_click(cx.listener(move |this, _, _, cx| {
                this.start_install(i);
                cx.notify();
            })));
        }
        d = d.child(
            div().flex().gap_2()
                .child(stat("hard-drive", t("size"), if total > 0 { fmt(total) } else { "—".into() }))
                .child(stat("calendar", t("updated"), a.release_date.clone().filter(|r| !r.is_empty()).unwrap_or("—".into()))),
        );
        if !desc.trim().is_empty() {
            d = d.child(section(t("description"), clip(desc, 900)));
        }
        if !a.screenshot_urls.is_empty() {
            d = d.child(div().flex().flex_col().gap_2().child(div().text_xs().text_color(rgb(DIM)).child("CAPTURAS")).child(
                div().id("shots").flex().gap_2().overflow_x_scroll().pb_2().children((0..a.screenshot_urls.len().min(8)).map(|n| {
                    let f = div().w(px(220.)).h(px(124.)).flex_none().rounded(px(10.)).overflow_hidden().bg(rgb(CARD)).border_1().border_color(rgb(LINE));
                    match images::shot(a, n) {
                        Some(p) => f.child(img(p).size_full().object_fit(ObjectFit::Cover)),
                        None => f,
                    }
                })),
            ));
        }
        if let Some(r) = a.requirements.as_deref().filter(|r| !r.trim().is_empty()) {
            d = d.child(section(t("requirements"), clip(r, 400)));
        }
        if let Some(c) = a.changelog.as_deref().filter(|c| !c.trim().is_empty()) {
            d = d.child(section(t("changelog"), clip(c, 600)));
        }
        d.child(Button::new("close").label(t("close")).on_click(cx.listener(|this, _, _, cx| {
            this.selected = None;
            cx.notify();
        })))
    }

    fn library(&self) -> AnyElement {
        let Some(v) = &self.vita else {
            return empty("gamepad-2", t("library-empty"));
        };
        let by_tid: std::collections::HashMap<&str, &api::App> =
            self.apps.iter().filter_map(|a| Some((a.title_id.as_deref()?, a))).collect();
        div()
            .id("lib")
            .p_4()
            .gap_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .size_full()
            .child(div().text_color(rgb(DIM)).child(format!(
                "{} · {} · {}",
                v.root.display(),
                v.auth.as_ref().map(|a| format!("{} ({})", a.username, a.role)).unwrap_or_else(|| t("no-session")),
                v.client_id.as_deref().map(|c| format!("client {}", &c[..c.len().min(8)])).unwrap_or_default()
            )))
            .children(v.apps.iter().map(|a| {
                let cat = by_tid.get(a.title_id.as_str());
                let tag = match (a.state.as_deref(), cat) {
                    (Some("outdated"), _) => (t("state-update"), WARN),
                    (_, Some(_)) => (t("state-catalog"), OK),
                    _ => (t("state-local"), DIM),
                };
                div().flex().items_center().gap_3().px_3().py_2().rounded(px(10.)).bg(rgb(CARD))
                    .child(div().flex_1().child(a.title.clone()))
                    .child(div().text_xs().text_color(rgb(DIM)).child(a.title_id.clone()))
                    .child(div().text_xs().text_color(rgb(DIM)).child(a.version.clone().unwrap_or_default()))
                    .child(chip(tag.0, tag.1))
            }))
            .into_any_element()
    }

    fn transfers_view(&self) -> AnyElement {
        if self.transfers.is_empty() {
            return empty("download", t("transfers-empty"));
        }
        div().p_4().gap_2().flex().flex_col().size_full().children(self.transfers.iter().rev().map(|tr| {
            let (d, tot) = (tr.progress.done.load(Relaxed), tr.progress.total.load(Relaxed));
            let line = match tr.result.lock().unwrap().clone() {
                Some(m) => m,
                None if tot > 0 => format!("{} / {} ({}%)", size(d), size(tot), d * 100 / tot),
                None => t1("downloaded", &[("size", size(d))]),
            };
            let done = tr.result.lock().unwrap().clone();
            let (frac, col) = match &done {
                Some(m) if m.starts_with(&t("error")) => (1.0, ERR),
                Some(_) => (1.0, OK),
                None if tot > 0 => (d as f32 / tot as f32, ACCENT),
                None => (0.0, ACCENT),
            };
            div().p_3().gap_2().flex().flex_col().rounded(px(12.)).bg(rgb(CARD)).child(tr.name.clone())
                .child(progress_bar(frac, col))
                .child(div().text_sm().text_color(rgb(DIM)).child(line))
        }))
        .into_any_element()
    }
}

impl Render for Main {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loading {
            return self.splash();
        }
        let vita_line: SharedString = match &self.vita {
            Some(v) => t1("vita-status", &[("user", v.auth.as_ref().map(|a| a.username.clone()).unwrap_or_else(|| t("no-session"))), ("count", v.apps.len().to_string())]).into(),
            None => t("vita-none").into(),
        };
        let body = match self.tab {
            Tab::Catalog if self.cat.is_none() && self.query.read(cx).value().is_empty() => self.home(cx),
            Tab::Catalog => self.grid(w, cx).into_any_element(),
            Tab::Library => self.library(),
            Tab::Transfers => self.transfers_view(),
            Tab::Settings => self.settings(cx),
        };
        let (title, count): (String, String) = match self.tab {
            Tab::Catalog => (self.cat.clone().unwrap_or_else(|| t("home")), t1("apps", &[("count", self.filtered.len().to_string())])),
            Tab::Library => (t("library"), t1("apps", &[("count", self.vita.as_ref().map_or(0, |v| v.apps.len()).to_string())])),
            Tab::Transfers => (t("transfers"), self.transfers.len().to_string()),
            Tab::Settings => (t("settings"), String::new()),
        };
        div()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .flex()
            .flex_col()
            .child(
                div().flex_1().flex().overflow_hidden()
                    .child(self.rail(cx))
                    .child(
                        div().flex_1().flex().flex_col().overflow_hidden()
                            .child(
                                div().h(px(80.)).px_6().flex().items_center().gap_3()
                                    .child(div().text_3xl().child(title))
                                    .child(div().text_sm().text_color(rgb(DIM)).child(count))
                                    .child(div().flex_1())
                                    .child(
                                        chip(i18n::current().code().to_uppercase(), ACCENT).id("lang").cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                i18n::cycle();
                                                this.status = "".into();
                                                cx.notify();
                                            })),
                                    )
                                    .child(chip(vita_line, if self.vita.is_some() { OK } else { DIM }))
                                    .child(div().w(px(300.)).child(Input::new(&self.query))),
                            )
                            .children((self.tab == Tab::Catalog).then(|| self.chips(cx)))
                            .child(div().flex_1().flex().flex_col().overflow_hidden().child(body)),
                    )
                    .children(self.selected.filter(|_| self.tab == Tab::Catalog).map(|i| self.detail(i, cx))),
            )
            .child(
                div().h(px(36.)).px_6().flex().items_center().gap_6().border_t_1().border_color(rgb(LINE)).bg(rgb(HEADER)).text_xs().text_color(rgb(DIM))
                    .child(t("shortcut-click")).child(div().flex().items_center().gap_1().child(icon("gamepad-2", 14., DIM)).child(t("find-vita")))
                    .child(div().flex_1())
                    .child(div().text_color(rgb(OK)).child("●"))
                    .child(format!("{} · {}", self.status, t1("apps", &[("count", self.filtered.len().to_string())]))),
            )
            .with_animation("enter", Animation::new(Duration::from_millis(350)), |e, p| e.opacity(p))
            .into_any_element()
    }
}

fn main() {
    dotenvy::dotenv().ok();
    i18n::init();
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |w, cx| cx.new(|cx| Main::new(w, cx))).expect("open window");
    });
}
