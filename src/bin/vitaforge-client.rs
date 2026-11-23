use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use std::{
    collections::HashSet,
    sync::{atomic::Ordering::Relaxed, Arc, Mutex},
    time::{Duration, Instant},
};
use vitaforge_client::{api, cache, download::Progress, images, install, vita};

const BG: u32 = 0x050a15;
const HEADER: u32 = 0x0a1224;
const CARD: u32 = 0x142038;
const HOVER: u32 = 0x203152;
const TEXT: u32 = 0xf4f7fb;
const DIM: u32 = 0x8298bc;
const ACCENT: u32 = 0x3890ff;
const OK: u32 = 0x30d158;
const WARN: u32 = 0xffd60a;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Catalog,
    Library,
    Transfers,
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
    cats: Vec<(String, usize)>,
    cat: Option<String>,
    query: Entity<InputState>,
    tab: Tab,
    selected: Option<usize>,
    vita: Option<vita::Vita>,
    transfers: Vec<Transfer>,
    status: SharedString,
    _sub: Subscription,
}

fn size(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

impl Main {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Buscar…"));
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
            apps: vec![],
            cats: vec![],
            cat: None,
            query,
            tab: Tab::Catalog,
            selected: None,
            vita: vita::find(),
            transfers: vec![],
            status: "".into(),
            _sub,
        }
    }

    fn set_catalog(&mut self, mut apps: Vec<api::App>, cx: &mut Context<Self>) {
        // PS1/PSP entries are not offered (same as the Vita app; their art is behind a bot challenge)
        apps.retain(|a| !matches!(a.kind.as_deref(), Some("psx_game" | "psp_game" | "psp_app")));
        let mut counts = std::collections::BTreeMap::new();
        for a in &apps {
            *counts.entry(a.category.clone().unwrap_or_else(|| "Other".into())).or_insert(0usize) += 1;
        }
        self.cats = counts.into_iter().collect();
        self.cats.sort_by(|a, b| b.1.cmp(&a.1));
        self.status = match (apps.is_empty(), api::base()) {
            (_, Err(e)) => e.into(),
            (true, _) => "Sin conexión al catálogo".into(),
            _ => "Catálogo cargado".into(),
        };
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
                self.cat.as_ref().is_none_or(|c| a.category.as_deref().unwrap_or("Other") == c)
                    && (q.is_empty() || a.name.to_lowercase().contains(&q))
            })
            .collect();
        cx.notify();
    }

    fn start_install(&mut self, i: usize) {
        let Some(root) = self.vita.as_ref().map(|v| v.root.clone()) else {
            self.status = "Conecta la Vita por USB primero".into();
            return;
        };
        let app = self.apps[i].clone();
        let t = Transfer { name: app.name.clone(), progress: Arc::default(), result: Arc::default() };
        let (p, r) = (t.progress.clone(), t.result.clone());
        std::thread::spawn(move || {
            let msg = match install::install(&app, &root, &p) {
                Ok(m) => m,
                Err(e) => format!("Error: {e}"),
            };
            *r.lock().unwrap() = Some(msg);
        });
        self.transfers.push(t);
        self.tab = Tab::Transfers;
    }

    fn rescan(&mut self) {
        self.vita = vita::find();
        self.status = if self.vita.is_some() { "Vita detectada".into() } else { "No se encontró la Vita".into() };
    }

    fn tab_btn(&self, id: &'static str, label: &'static str, t: Tab, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(id)
            .px_3()
            .py_1()
            .rounded(px(8.))
            .cursor_pointer()
            .bg(rgb(if self.tab == t { ACCENT } else { HEADER }))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.tab = t;
                cx.notify();
            }))
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |id: String, label: String, n: usize, sel: bool, cat: Option<String>, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(id))
                .px_2()
                .py_1()
                .rounded(px(6.))
                .cursor_pointer()
                .flex()
                .justify_between()
                .bg(rgb(if sel { CARD } else { BG }))
                .hover(|s| s.bg(rgb(HOVER)))
                .child(label)
                .child(div().text_color(rgb(DIM)).child(n.to_string()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cat = cat.clone();
                    this.tab = Tab::Catalog;
                    this.refilter(cx);
                }))
        };
        div()
            .id("sidebar")
            .w(px(190.))
            .p_2()
            .gap_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(rgb(HEADER))
            .child(item("cat-all".into(), "Todo".into(), self.apps.len(), self.cat.is_none(), None, cx))
            .children(self.cats.iter().map(|(c, n)| {
                item(format!("cat-{c}"), c.clone(), *n, self.cat.as_ref() == Some(c), Some(c.clone()), cx)
            }))
    }

    /// Cover width/height: PS Vita art measured at 0.78; entries with only an icon are square.
    fn ratio(a: &api::App) -> f32 {
        if a.cover_url.is_some() { 0.78 } else { 1.0 }
    }

    fn grid(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        const GAP: f32 = 12.;
        const PAD: f32 = 12.;
        const SIDEBAR: f32 = 190.;
        const TARGET: f32 = 170.;
        let avail = window.viewport_size().width.as_f32() - SIDEBAR - if self.selected.is_some() { 320. } else { 0. } - 2. * PAD;
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
                                let mut cover = div().w(px(slot_w)).h(px(h)).rounded(px(12.)).bg(rgb(CARD)).overflow_hidden()
                                    .flex().items_center().justify_center();
                                match images::path(a) {
                                    Some(p) => cover = cover.child(img(p).size_full().object_fit(ObjectFit::Cover)),
                                    None => {
                                        let tag = match a.kind.as_deref() {
                                            Some("psv_game") => "VITA",
                                            Some("plugin") => "PLUGIN",
                                            _ => "APP",
                                        };
                                        cover = cover.child(div().text_color(rgb(DIM)).child(tag));
                                    }
                                }
                                if a.title_id.as_deref().is_some_and(|t| have.contains(t)) {
                                    cover = cover.border_2().border_color(rgb(OK));
                                }
                                div()
                                    .id(("card", i))
                                    .w(px(slot_w))
                                    .h(px(slot_h + 40.))
                                    .cursor_pointer()
                                    .rounded(px(14.))
                                    .hover(|s| s.bg(rgb(HOVER)))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().w(px(slot_w)).h(px(slot_h)).flex().items_center().justify_center().child(cover))
                                    .child(div().text_sm().overflow_hidden().max_h(px(36.)).child(a.name.clone()))
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
        let mut d = div()
            .id("detail")
            .w(px(320.))
            .p_4()
            .gap_2()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(rgb(HEADER))
            .child(div().text_xl().child(a.name.clone()))
            .child(div().text_color(rgb(DIM)).child(format!(
                "{} · {} · v{}",
                a.author.clone().unwrap_or_default(),
                a.category.clone().unwrap_or_default(),
                a.version.clone().unwrap_or_default()
            )))
            .child(div().text_color(rgb(WARN)).child(format!(
                "★ {:.1} · {} instalaciones",
                a.average_rating.unwrap_or(0.0),
                a.install_count.unwrap_or(0)
            )));
        if a.download_url.is_some() || a.data_url.is_some() {
            d = d.child(Button::new("install").label("Instalar en la Vita").on_click(cx.listener(move |this, _, _, cx| {
                this.start_install(i);
                cx.notify();
            })));
        }
        d.child(Button::new("close").label("Cerrar").on_click(cx.listener(|this, _, _, cx| {
            this.selected = None;
            cx.notify();
        })))
    }

    fn library(&self) -> AnyElement {
        let Some(v) = &self.vita else {
            return div().p_4().text_color(rgb(DIM)).child("Conecta la Vita por USB (modo USB de VitaShell) y pulsa «Buscar Vita».").into_any_element();
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
                v.auth.as_ref().map(|a| format!("{} ({})", a.username, a.role)).unwrap_or_else(|| "sin sesión".into()),
                v.client_id.as_deref().map(|c| format!("client {}", &c[..c.len().min(8)])).unwrap_or_default()
            )))
            .children(v.apps.iter().map(|a| {
                let cat = by_tid.get(a.title_id.as_str());
                let tag = match (a.state.as_deref(), cat) {
                    (Some("outdated"), _) => ("actualización", WARN),
                    (_, Some(_)) => ("en catálogo", OK),
                    _ => ("local", DIM),
                };
                div().flex().gap_3().child(div().w(px(110.)).text_color(rgb(DIM)).child(a.title_id.clone()))
                    .child(div().flex_1().child(a.title.clone()))
                    .child(div().text_color(rgb(DIM)).child(a.version.clone().unwrap_or_default()))
                    .child(div().w(px(110.)).text_color(rgb(tag.1)).child(tag.0))
            }))
            .into_any_element()
    }

    fn transfers_view(&self) -> impl IntoElement {
        div().p_4().gap_2().flex().flex_col().size_full().children(self.transfers.iter().rev().map(|t| {
            let (d, tot) = (t.progress.done.load(Relaxed), t.progress.total.load(Relaxed));
            let line = match t.result.lock().unwrap().clone() {
                Some(m) => m,
                None if tot > 0 => format!("{} / {} ({}%)", size(d), size(tot), d * 100 / tot),
                None => format!("{} descargados…", size(d)),
            };
            div().p_2().rounded(px(8.)).bg(rgb(CARD)).child(t.name.clone()).child(div().text_sm().text_color(rgb(DIM)).child(line))
        }))
    }
}

impl Render for Main {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loading {
            return self.splash();
        }
        let vita_line: SharedString = match &self.vita {
            Some(v) => format!("Vita: {} · {} apps", v.auth.as_ref().map(|a| a.username.as_str()).unwrap_or("sin sesión"), v.apps.len()).into(),
            None => "Vita no detectada".into(),
        };
        let body = match self.tab {
            Tab::Catalog => self.grid(w, cx).into_any_element(),
            Tab::Library => self.library(),
            Tab::Transfers => self.transfers_view().into_any_element(),
        };
        div()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .flex()
            .flex_col()
            .child(
                div().h(px(52.)).px_4().bg(rgb(HEADER)).flex().items_center().gap_3()
                    .child(img(self.logo.clone()).size(px(30.)).rounded(px(7.)))
                    .child(div().text_color(rgb(ACCENT)).child("VitaForge"))
                    .child(self.tab_btn("t-cat", "Catálogo", Tab::Catalog, cx))
                    .child(self.tab_btn("t-lib", "Biblioteca", Tab::Library, cx))
                    .child(self.tab_btn("t-tr", "Transferencias", Tab::Transfers, cx))
                    .child(div().w(px(220.)).child(Input::new(&self.query)))
                    .child(div().flex_1())
                    .child(div().text_color(rgb(DIM)).child(vita_line))
                    .child(Button::new("rescan").label("Buscar Vita").on_click(cx.listener(|this, _, _, cx| {
                        this.rescan();
                        cx.notify();
                    }))),
            )
            .child(
                div().flex_1().flex().overflow_hidden()
                    .child(self.sidebar(cx))
                    .child(div().flex_1().flex().flex_col().child(body))
                    .children(self.selected.map(|i| self.detail(i, cx))),
            )
            .child(div().h(px(28.)).px_4().text_xs().text_color(rgb(DIM)).child(format!("{} · {} mostradas", self.status, self.filtered.len())))
            .with_animation("enter", Animation::new(Duration::from_millis(350)), |e, p| e.opacity(p))
            .into_any_element()
    }
}

fn main() {
    dotenvy::dotenv().ok();
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |w, cx| cx.new(|cx| Main::new(w, cx))).expect("open window");
    });
}
