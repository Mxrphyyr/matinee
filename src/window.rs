//! The main window: channels on the left, the feed on the right, and the
//! logic that ties feeds, search, playback and storage together.
//!
//! The feed is a virtualized grid: GTK keeps widgets only for cards near the
//! screen and recycles them while scrolling, and thumbnails are fetched only
//! for cards about to be seen. Long feeds stay light and smooth.

use crate::card::{self, Card, CardOpts};
use crate::model::{Channel, Video};
use crate::store::{Store, WindowState};
use crate::thumbs::Thumbs;
use crate::watch::{self, Watch, WatchHooks};
use crate::{feed, history, icons, perf, player, resolve, search, smooth, style, tools, trending, util, ytdlp, APP_ID};
use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// Smallest card width for the Small, Medium and Large card sizes.
const CARD_SIZES: [i32; 3] = [200, 248, 320];
const SPACING: i32 = 22;
const SIDE_PADDING: i32 = 32;
const DEFAULT_REFRESH_MINUTES: u32 = 20;
const SEARCH_RESULTS: usize = 40;
/// Changes are written to disk at most this often.
const SAVE_DELAY: Duration = Duration::from_millis(700);
/// Cards bound this soon after a feed appears play the entrance.
const ENTRANCE_WINDOW_US: i64 = 600_000;
/// Thumbnail requests are recalculated at most this often while scrolling.
const VISIBILITY_INTERVAL_US: i64 = 80_000;
/// A long list goes into the grid a slice at a time between frames; this
/// many items take about a millisecond.
const GRID_SLICE: usize = 200;
/// Channel rows added per idle moment: about 4 ms of work, one frame at
/// 240 Hz.
const SIDEBAR_SLICE: usize = 24;
const LOGO_SVG: &[u8] = include_bytes!("../data/io.github.matinee.Matinee.svg");
const QUALITIES: [(Option<u32>, &str); 6] = [
    (None, "Match my screen"),
    (Some(2160), "4K (2160p)"),
    (Some(1440), "1440p"),
    (Some(1080), "1080p"),
    (Some(720), "720p"),
    (Some(480), "480p"),
];
const REFRESH_CHOICES: [(u32, &str); 4] = [
    (10, "Every 10 minutes"),
    (20, "Every 20 minutes"),
    (60, "Every hour"),
    (0, "Only when I refresh"),
];

#[derive(Clone, PartialEq)]
enum View {
    Feed,
    Trending,
    Channel(String),
    Search(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Unwatched,
    ThisWeek,
}

#[derive(Clone, Copy)]
enum Fetch {
    Refresh { manual: bool },
    Added,
}

#[derive(Clone, PartialEq)]
enum RowTarget {
    All,
    Trending,
    Section,
    Channel(String),
}

/// What the channel list shows: whether the Channels section is folded, and
/// any filter text. Read by the list's filter function.
#[derive(Default)]
struct SidebarFilter {
    collapsed: Cell<bool>,
    query: RefCell<String>,
    targets: RefCell<Vec<RowTarget>>,
    names: RefCell<Vec<String>>,
}

pub struct Win {
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    nav: adw::NavigationView,
    content_view: adw::ToolbarView,
    channel_list: gtk::ListBox,
    channel_filter: gtk::SearchEntry,
    sidebar_filter: Rc<SidebarFilter>,
    section_row: RefCell<Option<gtk::ListBoxRow>>,
    search_entry: gtk::SearchEntry,
    refresh_button: gtk::Button,
    stack: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    grid: gtk::GridView,
    model: gio::ListStore,
    note: gtk::Label,
    hero: gtk::Box,
    hero_avatar: adw::Avatar,
    hero_title: gtk::Label,
    hero_sub: gtk::Label,
    hero_action: gtk::Button,
    filters: gtk::Box,
    filter_buttons: Vec<(Filter, gtk::ToggleButton)>,
    loading_title: gtk::Label,
    loading_sub: gtk::Label,
    empty: adw::StatusPage,
    empty_actions: gtk::Box,
    to_top: gtk::Revealer,
    new_pill: gtk::Revealer,
    new_pill_button: gtk::Button,
    watch: Rc<Watch>,

    thumbs: Rc<Thumbs>,
    store: Rc<RefCell<Store>>,
    save_source: Rc<RefCell<Option<glib::SourceId>>>,
    view: RefCell<View>,
    filter: Cell<Filter>,
    results: RefCell<Vec<Video>>,
    search_error: RefCell<Option<String>>,
    /// The search in progress, reading results as they arrive.
    search_stream: RefCell<Option<ytdlp::Stream>>,
    /// The current view's videos after filtering, in display order.
    shown: RefCell<Vec<Video>>,
    /// Items still to go into the grid, a slice per idle moment.
    grid_queue: RefCell<VecDeque<glib::BoxedAnyObject>>,
    grid_pumping: Cell<bool>,
    card_min: Cell<i32>,
    /// Every card the grid has made, keyed by its widget.
    cards: RefCell<HashMap<gtk::Button, Rc<Card>>>,
    /// Cards currently showing each video, for live updates.
    bound: RefCell<HashMap<String, Weak<Card>>>,
    entrance_until: Cell<i64>,
    /// Videos that arrived since the last visit. Kept for the whole session.
    new_ids: RefCell<HashSet<String>>,
    /// Just-added channels, whose existing uploads shouldn't count as new.
    quiet_channels: RefCell<HashSet<String>>,
    sidebar_badges: RefCell<Vec<(RowTarget, gtk::Label)>>,
    /// Bumped by every rebuild of the channel list, so slices of an older
    /// one stop.
    sidebar_generation: Cell<u64>,
    /// Older uploads per channel, read from yt-dlp this session.
    history: RefCell<HashMap<String, Vec<Video>>>,
    history_done: RefCell<HashSet<String>>,
    history_stream: RefCell<Option<(String, ytdlp::Stream)>>,
    /// More older uploads have been asked for and haven't arrived yet.
    history_asked: Cell<bool>,
    refreshing: Cell<bool>,
    busy: Cell<u32>,
    search_gen: Cell<u64>,
    suppress_select: Cell<bool>,
    hero_flip: Cell<bool>,
    header_raised: Cell<bool>,
    visibility_queued: Cell<bool>,
    last_visibility: Cell<i64>,
    scroll_animation: RefCell<Option<adw::TimedAnimation>>,
}

impl Win {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let store = Store::load();
        let card_min = CARD_SIZES[usize::from(store.card_size.unwrap_or(1).min(2))];
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Matinee")
            .default_width(1320)
            .default_height(860)
            .width_request(360)
            .height_request(480)
            .build();
        if let Some(saved) = store.window {
            window.set_default_size(saved.width.max(360), saved.height.max(480));
            if saved.maximized {
                window.maximize();
            }
        }

        // ── Sidebar ──
        let brand = gtk::Label::new(Some("Matinee"));
        brand.add_css_class("brand");
        let byline = gtk::Label::new(Some("By Mxrphyy"));
        byline.add_css_class("brand-by");
        let brand_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        brand_box.set_valign(gtk::Align::Center);
        brand_box.append(&brand);
        brand_box.append(&byline);
        let side_header = adw::HeaderBar::new();
        side_header.set_title_widget(Some(&brand_box));
        side_header.set_show_end_title_buttons(false);
        let add_button = gtk::Button::builder()
            .icon_name(icons::get("list-add-symbolic"))
            .tooltip_text("Add a channel")
            .action_name("win.add-channel")
            .build();
        side_header.pack_start(&add_button);

        let menu = gio::Menu::new();
        let section = gio::Menu::new();
        section.append(Some("_Import subscriptions…"), Some("win.import"));
        section.append(Some("_Hide Shorts"), Some("win.hide-shorts"));
        menu.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some("_Preferences"), Some("win.preferences"));
        section.append(Some("_About Matinee"), Some("win.about"));
        menu.append_section(None, &section);
        let menu_button = gtk::MenuButton::builder()
            .icon_name(icons::get("open-menu-symbolic"))
            .menu_model(&menu)
            .tooltip_text("Main menu")
            .primary(true)
            .build();
        side_header.pack_end(&menu_button);

        let channel_filter = gtk::SearchEntry::builder()
            .placeholder_text("Filter channels")
            .build();
        channel_filter.add_css_class("channel-filter");
        let channel_list = gtk::ListBox::new();
        channel_list.add_css_class("navigation-sidebar");
        channel_list.add_css_class("channel-list");
        let side_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&channel_list)
            .build();
        smooth::attach(&side_scroller);
        let side_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side_content.append(&channel_filter);
        side_content.append(&side_scroller);
        let side_view = adw::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&side_content));

        // ── Header ──
        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text("Search YouTube")
            .max_width_chars(44)
            .build();
        let content_header = adw::HeaderBar::new();
        content_header.set_title_widget(Some(&search_entry));
        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name(icons::get("sidebar-show-symbolic"))
            .tooltip_text("Channels (F9)")
            .build();
        content_header.pack_start(&sidebar_toggle);
        let refresh_button = gtk::Button::builder()
            .icon_name(icons::get("view-refresh-symbolic"))
            .tooltip_text("Refresh")
            .action_name("win.refresh")
            .build();
        content_header.pack_end(&refresh_button);

        // ── Greeting and filters ──
        let hero_avatar = adw::Avatar::new(52, None, true);
        hero_avatar.set_visible(false);
        hero_avatar.set_valign(gtk::Align::Center);
        let hero_title = label("hero-title");
        hero_title.set_wrap(true);
        let hero_sub = label("hero-sub");
        hero_sub.set_wrap(true);
        let hero_text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        hero_text.set_hexpand(true);
        hero_text.set_valign(gtk::Align::Center);
        hero_text.append(&hero_title);
        hero_text.append(&hero_sub);
        let hero_action = gtk::Button::builder()
            .label("Unsubscribe")
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let hero_top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        hero_top.append(&hero_avatar);
        hero_top.append(&hero_text);
        hero_top.append(&hero_action);

        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        filters.add_css_class("filters");
        let mut filter_buttons: Vec<(Filter, gtk::ToggleButton)> = Vec::new();
        for (filter, text) in [(Filter::All, "All"), (Filter::Unwatched, "Unwatched"), (Filter::ThisWeek, "This week")] {
            let button = gtk::ToggleButton::with_label(text);
            match filter_buttons.first() {
                Some((_, first)) => button.set_group(Some(first)),
                None => button.set_active(true),
            }
            filters.append(&button);
            filter_buttons.push((filter, button));
        }
        let hero = gtk::Box::new(gtk::Orientation::Vertical, 0);
        hero.add_css_class("hero");
        hero.append(&hero_top);
        hero.append(&filters);

        // ── Feed ──
        let model = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        let grid = gtk::GridView::new(Some(gtk::NoSelection::new(Some(model.clone()))), Some(factory.clone()));
        grid.add_css_class("video-grid");
        grid.set_min_columns(1);
        // Until the first layout, cap the columns at what the window can fit.
        let estimate = (window.default_size().0 - 280 - 2 * (SIDE_PADDING - SPACING / 2)) / card_min;
        grid.set_max_columns(estimate.clamp(1, 8) as u32);
        grid.set_vscroll_policy(gtk::ScrollablePolicy::Natural);
        // Item padding supplies the gaps; the margin tops it up to the page edge.
        grid.set_margin_start(SIDE_PADDING - SPACING / 2);
        grid.set_margin_end(SIDE_PADDING - SPACING / 2);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&grid)
            .build();
        smooth::attach(&scroller);
        let note = label("list-note");
        note.set_halign(gtk::Align::Center);
        note.set_valign(gtk::Align::Start);
        note.set_visible(false);

        let to_top_button = gtk::Button::builder()
            .icon_name(icons::get("go-top-symbolic"))
            .tooltip_text("Back to top")
            .build();
        to_top_button.add_css_class("fab");
        let to_top = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(280)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .child(&to_top_button)
            .build();
        let new_pill_button = gtk::Button::new();
        new_pill_button.add_css_class("new-pill");
        let new_pill = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(320)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .child(&new_pill_button)
            .build();
        let grid_overlay = gtk::Overlay::new();
        grid_overlay.set_child(Some(&scroller));
        grid_overlay.add_overlay(&note);
        grid_overlay.add_overlay(&to_top);
        grid_overlay.add_overlay(&new_pill);

        // ── Loading ──
        let loading_title = label("hero-title");
        let loading_sub = label("hero-sub");
        let loading_hero = gtk::Box::new(gtk::Orientation::Vertical, 4);
        loading_hero.add_css_class("hero");
        loading_hero.append(&loading_title);
        loading_hero.append(&loading_sub);
        let skeletons = skeleton_grid();
        for i in 0..8 {
            skeletons.append(&skeleton_card(i, card_min));
        }
        let loading_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        loading_page.append(&loading_hero);
        loading_page.append(&skeletons);
        let loading_scroller = natural_scroller(&loading_page);

        // ── Empty ──
        let empty = adw::StatusPage::new();
        empty.add_css_class("floaty");
        let import_button = gtk::Button::builder()
            .label("Import subscriptions…")
            .action_name("win.import")
            .build();
        import_button.add_css_class("pill");
        import_button.add_css_class("suggested-action");
        let add_channel_button = gtk::Button::builder()
            .label("Add a channel…")
            .action_name("win.add-channel")
            .build();
        add_channel_button.add_css_class("pill");
        let empty_actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
        empty_actions.set_halign(gtk::Align::Center);
        empty_actions.set_homogeneous(true);
        empty_actions.append(&import_button);
        empty_actions.append(&add_channel_button);
        empty.set_child(Some(&empty_actions));

        // No cross-fade: it would render two full pages per frame, and the
        // feed brings its own entrance.
        let stack = gtk::Stack::new();
        stack.add_named(&grid_overlay, Some("grid"));
        stack.add_named(&loading_scroller, Some("loading"));
        stack.add_named(&empty, Some("empty"));

        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        // The greeting and filters ride along under the header bar, so the
        // filters are always one click away and the shadow sits below both.
        content_view.add_top_bar(&hero);
        content_view.set_content(Some(&stack));

        // The channel list is a drawer: shown beside the feed on wide windows,
        // sliding over it on narrow ones, and hidden whenever you like.
        let split = adw::OverlaySplitView::builder()
            .sidebar(&side_view)
            .content(&content_view)
            .min_sidebar_width(230.0)
            .max_sidebar_width(300.0)
            .show_sidebar(!store.hide_sidebar)
            .build();
        split
            .bind_property("show-sidebar", &sidebar_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        // The browser is the root page; the player slides in over it.
        let browse_page = adw::NavigationPage::builder()
            .child(&split)
            .title("Matinee")
            .tag("browse")
            .build();
        let nav = adw::NavigationView::new();
        nav.add(&browse_page);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&nav));
        window.set_content(Some(&toasts));

        let breakpoint = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 760sp").expect("valid breakpoint"),
        );
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        breakpoint.add_setter(&split, "show-sidebar", Some(&false.to_value()));
        window.add_breakpoint(breakpoint);

        let volume = store.volume.unwrap_or(100.0);
        let quality = store.quality;
        let sidebar_filter = Rc::new(SidebarFilter::default());
        sidebar_filter.collapsed.set(store.channels_collapsed);
        let win = Rc::new_cyclic(|weak: &Weak<Self>| {
            let watch = Watch::new(volume, quality, watch_hooks(weak));
            nav.add(&watch.page);
            Self {
                window,
                toasts,
                split,
                nav,
                content_view,
                channel_list,
                channel_filter,
                sidebar_filter,
                section_row: RefCell::default(),
                search_entry,
                refresh_button,
                stack,
                scroller,
                grid,
                model,
                note,
                hero,
                hero_avatar,
                hero_title,
                hero_sub,
                hero_action,
                filters,
                filter_buttons,
                loading_title,
                loading_sub,
                empty,
                empty_actions,
                to_top,
                new_pill,
                new_pill_button,
                watch,
                thumbs: Thumbs::new(),
                store: Rc::new(RefCell::new(store)),
                save_source: Rc::default(),
                view: RefCell::new(View::Feed),
                filter: Cell::new(Filter::All),
                results: RefCell::default(),
                search_error: RefCell::default(),
                search_stream: RefCell::default(),
                shown: RefCell::default(),
                grid_queue: RefCell::default(),
                grid_pumping: Cell::new(false),
                card_min: Cell::new(card_min),
                cards: RefCell::default(),
                bound: RefCell::default(),
                entrance_until: Cell::new(0),
                new_ids: RefCell::default(),
                quiet_channels: RefCell::default(),
                sidebar_badges: RefCell::default(),
                sidebar_generation: Cell::new(0),
                history: RefCell::default(),
                history_done: RefCell::default(),
                history_stream: RefCell::default(),
                history_asked: Cell::new(false),
                refreshing: Cell::new(false),
                busy: Cell::new(0),
                search_gen: Cell::new(0),
                suppress_select: Cell::new(false),
                hero_flip: Cell::new(false),
                header_raised: Cell::new(false),
                visibility_queued: Cell::new(false),
                last_visibility: Cell::new(0),
                scroll_animation: RefCell::default(),
            }
        });
        win.connect_signals(&to_top_button, &factory);
        win.install_actions();
        win
    }

    /// Shows the window at once and fills in the feed right after its first
    /// frame, so the start never waits on the grid.
    pub fn start(self: &Rc<Self>) {
        let (always_dark, smooth_scrolling, sign_in, has_channels, has_feed, channels) = {
            let st = self.store.borrow();
            (
                st.always_dark,
                !st.no_smooth_scroll,
                st.cookies_browser.clone(),
                !st.channels.is_empty(),
                !st.feed.is_empty(),
                st.channels.len(),
            )
        };
        style::set_always_dark(always_dark);
        smooth::set_enabled(smooth_scrolling);
        tools::set_sign_in(sign_in.as_deref());
        if has_feed {
            // The greeting over the loading page, for the first frame only.
            let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
            self.loading_title.set_label(util::greeting(hour));
            self.loading_sub.set_label(&format!("The latest from {}", util::count(channels, "channel", "channels")));
            self.hero.set_visible(false);
            self.stack.set_visible_child_name("loading");
        }
        perf::watch_frames(&self.window);
        let weak = Rc::downgrade(self);
        self.window.add_tick_callback(move |_, _| {
            // The first frame is about to be drawn; the rest follows it.
            if let Some(win) = weak.upgrade() {
                glib::idle_add_local_once(move || win.fill_in(has_channels, has_feed));
            }
            glib::ControlFlow::Break
        });
        // Start the video engine in the background so the first video opens at once.
        let watch = Rc::downgrade(&self.watch);
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if let Some(watch) = watch.upgrade() {
                watch.warm_up();
            }
        });
        self.schedule_refresh();
        self.note_last_crash();
    }

    /// The feed, then the channel list, then a check for new uploads unless
    /// one was done only a little while ago.
    fn fill_in(self: &Rc<Self>, has_channels: bool, has_feed: bool) {
        perf::mark("first frame drawn");
        if has_feed || !has_channels {
            self.render();
        }
        // The channel list a moment later, so the feed's entrance isn't held
        // up by building hundreds of rows.
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(win) = weak.upgrade() {
                win.rebuild_sidebar();
            }
        });
        // Opening Matinee again soon after shouldn't ask YouTube for every channel again.
        let interval = self.refresh_interval().unwrap_or(i64::from(DEFAULT_REFRESH_MINUTES) * 60);
        if !has_feed || self.since_refresh() >= interval {
            self.refresh(false);
        }
    }

    /// Says where to find the details if Matinee stopped unexpectedly last
    /// time. The note shows once; the log is kept under another name.
    fn note_last_crash(&self) {
        let log = crate::crash_log();
        if !log.is_file() {
            return;
        }
        let kept = log.with_file_name("crash-previous.log");
        if std::fs::rename(&log, &kept).is_ok() {
            let toast = self.toast("Matinee stopped unexpectedly last time. The details are in ~/.cache/matinee/crash-previous.log");
            toast.set_timeout(12);
        }
    }

    fn connect_signals(self: &Rc<Self>, to_top_button: &gtk::Button, factory: &gtk::SignalListItemFactory) {
        // ── Feed cards ──
        let weak = Rc::downgrade(self);
        factory.connect_setup(move |_, object| {
            let (Some(win), Some(item)) = (weak.upgrade(), object.downcast_ref::<gtk::ListItem>()) else { return };
            // Grid items are just slots; focus and clicks belong to the cards.
            item.set_activatable(false);
            item.set_selectable(false);
            item.set_focusable(false);
            let card = Card::new(win.card_min.get() - SPACING);
            item.set_child(Some(&card.root));
            win.cards.borrow_mut().insert(card.root.clone(), card);
        });
        let weak = Rc::downgrade(self);
        factory.connect_bind(move |_, object| {
            let (Some(win), Some(item)) = (weak.upgrade(), object.downcast_ref::<gtk::ListItem>()) else { return };
            let Some(card) = win.card_of(item) else { return };
            let Some(data) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
            let video = data.borrow::<Video>().clone();
            win.bind_card(&card, &video, item.position());
        });
        let weak = Rc::downgrade(self);
        factory.connect_unbind(move |_, object| {
            let (Some(win), Some(item)) = (weak.upgrade(), object.downcast_ref::<gtk::ListItem>()) else { return };
            if let Some(card) = win.card_of(item) {
                win.release_card(&card);
            }
        });
        let weak = Rc::downgrade(self);
        factory.connect_teardown(move |_, object| {
            let (Some(win), Some(item)) = (weak.upgrade(), object.downcast_ref::<gtk::ListItem>()) else { return };
            if let Some(root) = item.child().and_downcast::<gtk::Button>() {
                let card = win.cards.borrow_mut().remove(&root);
                if let Some(card) = card {
                    win.release_card(&card);
                }
            }
        });

        // ── Window and player ──
        let this = self.clone();
        self.nav.connect_popped(move |_, page| {
            if page.tag().as_deref() == Some("watch") {
                // Playback continues if the picture moved to its own window.
                this.watch.page_closed();
                this.save_later();
                // Back at the feed: catch up if a check came due while watching.
                this.refresh_if_due();
            }
        });
        let this = self.clone();
        self.window.connect_close_request(move |window| {
            let (width, height) = window.default_size();
            this.store.borrow_mut().window = Some(WindowState { width, height, maximized: window.is_maximized() });
            this.stop_history();
            this.stop_search();
            this.watch.stop();
            this.save_now();
            glib::Propagation::Proceed
        });

        // ── Sidebar ──
        let filter = self.sidebar_filter.clone();
        self.channel_list.set_filter_func(move |row| {
            let index = row.index() as usize;
            match filter.targets.borrow().get(index) {
                Some(RowTarget::Channel(_)) => {
                    let query = filter.query.borrow();
                    if query.is_empty() {
                        !filter.collapsed.get()
                    } else {
                        filter.names.borrow().get(index).is_some_and(|name| name.contains(query.as_str()))
                    }
                }
                _ => true,
            }
        });
        let this = self.clone();
        self.channel_filter.connect_search_changed(move |entry| {
            *this.sidebar_filter.query.borrow_mut() = entry.text().trim().to_lowercase();
            this.channel_list.invalidate_filter();
        });
        let this = self.clone();
        self.channel_filter.connect_stop_search(move |entry| {
            entry.set_text("");
            this.channel_list.grab_focus();
        });
        let this = self.clone();
        self.channel_list.connect_row_selected(move |_, row| this.on_row_selected(row));
        let this = self.clone();
        self.channel_list.connect_row_activated(move |_, row| {
            let index = row.index() as usize;
            let target = this.sidebar_filter.targets.borrow().get(index).cloned();
            if target == Some(RowTarget::Section) {
                this.toggle_channels();
            } else {
                this.after_navigation();
            }
        });
        let this = self.clone();
        self.split.connect_show_sidebar_notify(move |split| {
            // Remember the choice made on a wide window; narrow windows hide it anyway.
            if !split.is_collapsed() {
                this.store.borrow_mut().hide_sidebar = !split.shows_sidebar();
                this.save_later();
            }
        });

        // ── Search ──
        let this = self.clone();
        self.search_entry
            .connect_activate(move |entry| this.start_search(entry.text().to_string()));
        let this = self.clone();
        self.search_entry.connect_stop_search(move |entry| {
            entry.set_text("");
            if this.in_search() {
                this.show_view(View::Feed);
            }
        });

        // ── Greeting, filters and floating buttons ──
        let this = self.clone();
        self.hero_action.connect_clicked(move |_| this.unsubscribe_current());
        for (filter, button) in &self.filter_buttons {
            let filter = *filter;
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |button| {
                let Some(win) = weak.upgrade() else { return };
                if button.is_active() && win.filter.get() != filter {
                    win.filter.set(filter);
                    win.render();
                }
            });
        }
        let this = self.clone();
        to_top_button.connect_clicked(move |_| this.scroll_to_top());
        let this = self.clone();
        self.new_pill_button
            .connect_clicked(move |_| this.show_view(View::Feed));

        // ── Scrolling ──
        let this = self.clone();
        self.scroller.vadjustment().connect_value_changed(move |adjustment| {
            card::note_scroll();
            this.to_top
                .set_reveal_child(adjustment.value() > adjustment.page_size() * 1.5);
            // A soft shadow under the header once the feed slides beneath it.
            let raised = adjustment.value() > 2.0;
            if raised != this.header_raised.replace(raised) {
                this.content_view.set_top_bar_style(if raised {
                    adw::ToolbarStyle::Raised
                } else {
                    adw::ToolbarStyle::Flat
                });
            }
            this.queue_visibility();
        });
        // Cap the columns at what fits, so the grid doesn't build cards for a
        // wider window than it has.
        let weak = Rc::downgrade(self);
        self.scroller.hadjustment().connect_page_size_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.grid.set_max_columns(win.columns() as u32);
            }
        });
        // The grid reconfigures the scroll range at the end of each layout, so
        // this is the moment its cards have real positions.
        let weak = Rc::downgrade(self);
        self.scroller.vadjustment().connect_changed(move |_| {
            if let Some(win) = weak.upgrade() {
                win.queue_visibility();
            }
        });
    }

    fn install_actions(self: &Rc<Self>) {
        self.action("refresh", |w| w.refresh(true));
        self.action("add-channel", |w| w.add_channel_dialog());
        self.action("import", |w| w.import_dialog());
        self.action("about", |w| w.about());
        self.action("preferences", |w| w.preferences());
        self.action("sign-in", |w| w.sign_in_dialog());
        self.action("toggle-sidebar", |w| w.split.set_show_sidebar(!w.split.shows_sidebar()));
        self.action("focus-search", |w| {
            w.after_navigation();
            w.search_entry.grab_focus();
        });
        self.action_with_id("search", |w, query| {
            w.search_entry.set_text(query);
            w.start_search(query.to_string());
        });
        self.action_with_id("play", |w, id| w.play(id));
        self.action_with_id("play-external", |w, id| {
            if let Some(video) = w.find_video(id) {
                w.play_external(&video, None);
            }
        });
        self.action_with_id("toggle-watched", |w, id| {
            let watched = w.store.borrow().watched.contains(id);
            w.set_watched(id, !watched);
        });
        self.action_with_id("copy-link", |w, id| w.copy_link(id));
        self.action_with_id("open-browser", |w, id| w.open_in_browser(id));
        self.action_with_id("subscribe", |w, id| w.subscribe_to_channel_of(id));
        self.action_with_id("show-channel", |w, id| w.show_view(View::Channel(id.to_string())));

        let (hide_shorts, always_dark, external) = {
            let st = self.store.borrow();
            (st.hide_shorts, st.always_dark, st.external_player)
        };
        self.toggle("hide-shorts", hide_shorts, |w, on| {
            w.store.borrow_mut().hide_shorts = on;
            w.save_later();
            w.update_sidebar_counts();
            w.render();
        });
        self.toggle("always-dark", always_dark, |w, on| {
            w.store.borrow_mut().always_dark = on;
            w.save_later();
            style::set_always_dark(on);
        });
        self.toggle("external-player", external, |w, on| {
            w.store.borrow_mut().external_player = on;
            w.save_later();
        });
    }

    fn toggle(self: &Rc<Self>, name: &str, initial: bool, f: impl Fn(&Rc<Self>, bool) + 'static) {
        let action = gio::SimpleAction::new_stateful(name, None, &initial.to_variant());
        let this = self.clone();
        action.connect_activate(move |action, _| {
            let on = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            action.set_state(&on.to_variant());
            f(&this, on);
        });
        self.window.add_action(&action);
    }

    /// Sets a toggle action to `on`, running its effects only if it changes.
    fn set_toggle(&self, name: &str, on: bool) {
        if let Some(action) = self.window.lookup_action(name) {
            if action.state().and_then(|s| s.get::<bool>()) != Some(on) {
                action.activate(None);
            }
        }
    }

    fn action(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        let action = gio::SimpleAction::new(name, None);
        let this = self.clone();
        action.connect_activate(move |_, _| f(&this));
        self.window.add_action(&action);
    }

    fn action_with_id(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>, &str) + 'static) {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let this = self.clone();
        action.connect_activate(move |_, param| {
            if let Some(id) = param.and_then(|p| p.get::<String>()) {
                f(&this, &id);
            }
        });
        self.window.add_action(&action);
    }

    // ── Saving and timers ──────────────────────────────────────────────

    /// Writes the state file soon, folding a burst of changes into one write.
    fn save_later(&self) {
        if self.save_source.borrow().is_some() {
            return;
        }
        let store = self.store.clone();
        let source = self.save_source.clone();
        let id = glib::timeout_add_local_once(SAVE_DELAY, move || {
            source.borrow_mut().take();
            store.borrow().save();
        });
        *self.save_source.borrow_mut() = Some(id);
    }

    fn save_now(&self) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
        }
        self.store.borrow().save();
    }

    /// Checks for new uploads when they're due, but only while the feed is
    /// in front of you: a window in the background, or a video playing, asks
    /// YouTube for nothing. Coming back to the feed catches up.
    fn schedule_refresh(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(60, move || match weak.upgrade() {
            Some(win) => {
                win.refresh_if_due();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.refresh_if_due();
            }
        });
    }

    fn refresh_if_due(self: &Rc<Self>) {
        let Some(interval) = self.refresh_interval() else { return };
        let browsing = self.nav.visible_page().and_then(|page| page.tag()).as_deref() == Some("browse");
        if browsing && self.window.is_active() && self.since_refresh() >= interval {
            self.refresh(false);
        }
    }

    /// Seconds between automatic checks, or None if they're turned off.
    fn refresh_interval(&self) -> Option<i64> {
        match self.store.borrow().refresh_minutes.unwrap_or(DEFAULT_REFRESH_MINUTES) {
            0 => None,
            minutes => Some(i64::from(minutes) * 60),
        }
    }

    /// Seconds since the feed was last checked for new uploads. A check
    /// "in the future" (the clock was wrong then) counts as long ago.
    fn since_refresh(&self) -> i64 {
        let age = util::now() - self.store.borrow().last_refresh.unwrap_or(0);
        if age < 0 {
            i64::MAX
        } else {
            age
        }
    }

    // ── Sidebar ────────────────────────────────────────────────────────

    /// On a narrow window the channel list slides over the feed; tuck it away
    /// once a choice is made.
    fn after_navigation(&self) {
        if self.split.is_collapsed() {
            self.split.set_show_sidebar(false);
        }
    }

    fn on_row_selected(self: &Rc<Self>, row: Option<&gtk::ListBoxRow>) {
        if self.suppress_select.get() {
            return;
        }
        let Some(row) = row else { return };
        let target = self.sidebar_filter.targets.borrow().get(row.index() as usize).cloned();
        let view = match target {
            Some(RowTarget::All) => View::Feed,
            Some(RowTarget::Trending) => View::Trending,
            Some(RowTarget::Channel(id)) => View::Channel(id),
            _ => return,
        };
        if *self.view.borrow() != view {
            self.show_view(view);
        }
    }

    fn select_row_for_view(&self) {
        let wanted = match &*self.view.borrow() {
            View::Feed => Some(RowTarget::All),
            View::Trending => Some(RowTarget::Trending),
            View::Channel(id) => Some(RowTarget::Channel(id.clone())),
            View::Search(_) => None,
        };
        let index = wanted.and_then(|wanted| {
            self.sidebar_filter
                .targets
                .borrow()
                .iter()
                .position(|t| *t == wanted)
        });
        self.suppress_select.set(true);
        match index.and_then(|i| self.channel_list.row_at_index(i as i32)) {
            Some(row) => self.channel_list.select_row(Some(&row)),
            None => self.channel_list.unselect_all(),
        }
        self.suppress_select.set(false);
    }

    /// Recreates the channel rows: the fixed rows at once, then the channels
    /// a few dozen per idle moment, so hundreds of them never stall a frame.
    /// Only needed when the channel list itself changes; new-video counts
    /// update in place.
    fn rebuild_sidebar(self: &Rc<Self>) {
        let _timing = perf::span("channel list", 0);
        let mut channels = self.store.borrow().channels.clone();
        channels.sort_by_cached_key(|c| c.name.to_lowercase());
        self.suppress_select.set(true);
        self.channel_list.remove_all();
        let mut targets = Vec::with_capacity(channels.len() + 3);
        let mut names = Vec::with_capacity(channels.len() + 3);
        let mut badges = Vec::with_capacity(channels.len() + 1);

        let (row, badge) = sidebar_row(RowIcon::Named("view-grid-symbolic"), "All videos");
        self.channel_list.append(&row);
        targets.push(RowTarget::All);
        names.push(String::new());
        badges.push((RowTarget::All, badge));

        let (row, _) = sidebar_row(RowIcon::Named("trending-symbolic"), "Trending");
        self.channel_list.append(&row);
        targets.push(RowTarget::Trending);
        names.push(String::new());

        let section = section_row(channels.len(), self.sidebar_filter.collapsed.get());
        self.channel_list.append(&section);
        targets.push(RowTarget::Section);
        names.push(String::new());
        *self.section_row.borrow_mut() = Some(section);

        *self.sidebar_filter.targets.borrow_mut() = targets;
        *self.sidebar_filter.names.borrow_mut() = names;
        *self.sidebar_badges.borrow_mut() = badges;
        self.channel_list.invalidate_filter();
        self.suppress_select.set(false);
        self.select_row_for_view();
        self.update_sidebar_counts();
        let generation = self.sidebar_generation.get() + 1;
        self.sidebar_generation.set(generation);
        self.add_channel_rows(Rc::new(channels), 0, generation);
    }

    fn add_channel_rows(self: &Rc<Self>, channels: Rc<Vec<Channel>>, from: usize, generation: u64) {
        // A newer rebuild has taken over.
        if generation != self.sidebar_generation.get() {
            return;
        }
        let _timing = perf::span("channel rows", 3);
        let to = (from + SIDEBAR_SLICE).min(channels.len());
        for channel in &channels[from..to] {
            // The row's place is on record before the row exists, so the
            // list's filter (which reads it as the row is added) finds it.
            self.sidebar_filter.targets.borrow_mut().push(RowTarget::Channel(channel.id.clone()));
            self.sidebar_filter.names.borrow_mut().push(channel.name.to_lowercase());
            let (row, badge) = sidebar_row(RowIcon::Avatar(&channel.name), &channel.name);
            self.sidebar_badges.borrow_mut().push((RowTarget::Channel(channel.id.clone()), badge));
            self.channel_list.append(&row);
        }
        if to < channels.len() {
            let weak = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                if let Some(win) = weak.upgrade() {
                    win.add_channel_rows(channels, to, generation);
                }
            });
        } else {
            self.select_row_for_view();
            self.update_sidebar_counts();
            perf::mark("channel list complete");
        }
    }

    /// Folds or unfolds the Channels section.
    fn toggle_channels(&self) {
        let collapsed = !self.sidebar_filter.collapsed.get();
        self.sidebar_filter.collapsed.set(collapsed);
        self.store.borrow_mut().channels_collapsed = collapsed;
        self.save_later();
        if let Some(row) = self.section_row.borrow().as_ref() {
            if collapsed {
                row.add_css_class("collapsed");
            } else {
                row.remove_css_class("collapsed");
            }
        }
        self.channel_list.invalidate_filter();
    }

    fn update_sidebar_counts(&self) {
        let (per_channel, total) = {
            let st = self.store.borrow();
            let new_ids = self.new_ids.borrow();
            let mut per_channel: HashMap<String, usize> = HashMap::new();
            let mut total = 0;
            for video in st
                .feed
                .iter()
                .filter(|v| new_ids.contains(&v.id) && !(st.hide_shorts && v.is_short))
            {
                *per_channel.entry(video.channel_id.clone()).or_default() += 1;
                total += 1;
            }
            (per_channel, total)
        };
        for (target, badge) in self.sidebar_badges.borrow().iter() {
            let count = match target {
                RowTarget::Channel(id) => per_channel.get(id).copied().unwrap_or(0),
                _ => total,
            };
            badge.set_label(&count.to_string());
            badge.set_tooltip_text(Some(&util::count(count, "new video", "new videos")));
            badge.set_visible(count > 0);
        }
    }

    // ── Views ──────────────────────────────────────────────────────────

    fn in_search(&self) -> bool {
        matches!(*self.view.borrow(), View::Search(_))
    }

    fn show_view(self: &Rc<Self>, view: View) {
        if !matches!(view, View::Search(_)) {
            self.stop_search();
            if !self.search_entry.text().is_empty() {
                self.search_entry.set_text("");
            }
        }
        *self.view.borrow_mut() = view;
        self.select_row_for_view();
        self.render();
        self.after_navigation();
    }

    /// A channel's videos: the feed's newest ones first (exact dates and
    /// views), then its older uploads as far as they've been read.
    fn channel_videos(&self, id: &str, st: &Store) -> Vec<Video> {
        let mut videos: Vec<Video> = st.feed.iter().filter(|v| v.channel_id == id).cloned().collect();
        if let Some(older) = self.history.borrow().get(id) {
            let known: HashSet<String> = videos.iter().map(|v| v.id.clone()).collect();
            videos.extend(older.iter().filter(|v| !known.contains(&v.id)).cloned());
        }
        videos
    }

    fn passes_filter(&self, video: &Video, st: &Store, now: i64) -> bool {
        if st.hide_shorts && video.is_short {
            return false;
        }
        if self.in_search() {
            return true;
        }
        match self.filter.get() {
            Filter::All => true,
            Filter::Unwatched => !st.watched.contains(&video.id),
            Filter::ThisWeek => video.published.is_some_and(|t| now - t < 7 * 86_400),
        }
    }

    /// Fills the feed for the current view and filter, with the entrance.
    fn render(self: &Rc<Self>) {
        let _timing = perf::span("render", 0);
        self.new_pill.set_reveal_child(false);
        let view = self.view.borrow().clone();
        // Older uploads are only read for the channel on screen.
        let keep_history = matches!(&view, View::Channel(id) if self.history_running(id));
        if !keep_history {
            self.stop_history();
        }
        let now = util::now();
        let (has_any, list) = {
            let st = self.store.borrow();
            let all: Vec<Video> = match &view {
                View::Feed => st.feed.clone(),
                View::Trending => trending::rank(&st.feed, now),
                View::Channel(id) => self.channel_videos(id, &st),
                View::Search(_) => self.results.borrow().clone(),
            };
            let has_any = all.iter().any(|v| !(st.hide_shorts && v.is_short));
            let list: Vec<Video> = all.into_iter().filter(|v| self.passes_filter(v, &st, now)).collect();
            (has_any, list)
        };
        if !matches!(view, View::Search(_)) {
            // Once the feed has been on screen, everything in it counts as seen.
            let changed = {
                let mut st = self.store.borrow_mut();
                let ids: HashSet<String> = st.feed.iter().map(|v| v.id.clone()).collect();
                let changed = st.seen != ids;
                st.seen = ids;
                changed
            };
            if changed {
                self.save_later();
            }
        }

        // A channel whose older uploads are still to come gets the grid anyway.
        let channel_pending = matches!(&view, View::Channel(id) if !self.history_done.borrow().contains(id));
        let items: Vec<glib::BoxedAnyObject> = list.iter().cloned().map(glib::BoxedAnyObject::new).collect();
        let nothing_here = list.is_empty();
        *self.shown.borrow_mut() = list;
        if !has_any && !channel_pending {
            self.grid_queue.borrow_mut().clear();
            self.model.remove_all();
            self.update_hero(true);
            self.show_empty(&view);
            return;
        }
        self.hero.set_visible(true);
        self.stack.set_visible_child_name("grid");
        self.note.set_visible(nothing_here);
        self.note.set_label(match (&view, self.filter.get()) {
            (View::Channel(_), Filter::All) => "Loading this channel's videos…",
            (View::Trending, Filter::All) => "Nothing is trending yet. Check back once your channels post.",
            (_, Filter::Unwatched) => "You've watched everything here.",
            (_, Filter::ThisWeek) => "Nothing was uploaded here this week.",
            _ => "Nothing to show.",
        });
        let animate = adw::is_animations_enabled(&self.grid);
        self.entrance_until
            .set(if animate { glib::monotonic_time() + ENTRANCE_WINDOW_US } else { 0 });
        self.scroller.vadjustment().set_value(0.0);
        self.fill_grid(items);
        if let View::Channel(id) = &view {
            if channel_pending && !self.history_running(id) {
                self.start_history(id, false);
            }
        }
        self.update_hero(true);
    }

    /// Puts `items` in the grid: enough for the first screens at once, the
    /// rest a slice at a time between frames, so a long feed never stalls.
    fn fill_grid(self: &Rc<Self>, items: Vec<glib::BoxedAnyObject>) {
        let first = (self.columns() * 10).clamp(60, 200).min(items.len());
        self.model.splice(0, self.model.n_items(), &items[..first]);
        {
            let mut queue = self.grid_queue.borrow_mut();
            queue.clear();
            queue.extend(items.into_iter().skip(first));
        }
        self.pump_grid();
    }

    /// Adds `items` after everything else, behind any slices still waiting.
    fn append_grid(self: &Rc<Self>, items: Vec<glib::BoxedAnyObject>) {
        if self.grid_queue.borrow().is_empty() {
            self.model.splice(self.model.n_items(), 0, &items);
        } else {
            self.grid_queue.borrow_mut().extend(items);
            self.pump_grid();
        }
    }

    fn pump_grid(self: &Rc<Self>) {
        if self.grid_queue.borrow().is_empty() || self.grid_pumping.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(win) = weak.upgrade() else { return };
            win.grid_pumping.set(false);
            let slice: Vec<glib::BoxedAnyObject> = {
                let mut queue = win.grid_queue.borrow_mut();
                let count = queue.len().min(GRID_SLICE);
                queue.drain(..count).collect()
            };
            if !slice.is_empty() {
                let _timing = perf::span("grid slice", 3);
                win.model.splice(win.model.n_items(), 0, &slice);
            }
            win.pump_grid();
        });
    }

    // ── Channel history ────────────────────────────────────────────────

    fn history_running(&self, id: &str) -> bool {
        self.history_stream
            .borrow()
            .as_ref()
            .is_some_and(|(running, _)| running == id)
    }

    /// Reads a channel's older uploads, with the browser sign-in only when
    /// YouTube has refused to answer without it.
    fn start_history(self: &Rc<Self>, id: &str, signed_in: bool) {
        self.stop_history();
        let name = self.store.borrow().channel_name(id).unwrap_or_default();
        // Without yt-dlp the channel page simply shows its newest videos.
        let Ok((stream, updates)) = history::start(id, &name, signed_in) else { return };
        *self.history_stream.borrow_mut() = Some((id.to_string(), stream));
        self.history_asked.set(false);
        let this = self.clone();
        let id = id.to_string();
        glib::spawn_future_local(async move {
            let mut failure = None;
            while let Ok(update) = updates.recv().await {
                match update {
                    ytdlp::Update::Videos(batch) => this.add_history(&id, batch),
                    ytdlp::Update::Failed(reason) => failure = Some(reason),
                }
            }
            this.finish_history(&id, failure);
        });
    }

    fn add_history(self: &Rc<Self>, id: &str, batch: Vec<Video>) {
        self.history_asked.set(false);
        let added: Vec<Video> = {
            let st = self.store.borrow();
            let mut history = self.history.borrow_mut();
            let older = history.entry(id.to_string()).or_default();
            let mut known: HashSet<String> = older.iter().map(|v| v.id.clone()).collect();
            known.extend(st.feed.iter().filter(|v| v.channel_id == id).map(|v| v.id.clone()));
            let fresh: Vec<Video> = batch.into_iter().filter(|v| known.insert(v.id.clone())).collect();
            older.extend(fresh.iter().cloned());
            fresh
        };
        if *self.view.borrow() != View::Channel(id.to_string()) {
            return;
        }
        let now = util::now();
        let visible: Vec<Video> = {
            let st = self.store.borrow();
            added.into_iter().filter(|v| self.passes_filter(v, &st, now)).collect()
        };
        if !visible.is_empty() {
            // Appended at the end, so the part of the grid you're looking at stays put.
            let items: Vec<glib::BoxedAnyObject> = visible.iter().cloned().map(glib::BoxedAnyObject::new).collect();
            self.append_grid(items);
            self.shown.borrow_mut().extend(visible);
            self.note.set_visible(false);
        }
        self.update_hero(false);
    }

    fn finish_history(self: &Rc<Self>, id: &str, failure: Option<String>) {
        let finished = {
            let mut slot = self.history_stream.borrow_mut();
            if slot.as_ref().is_some_and(|(running, _)| running == id) {
                slot.take()
            } else {
                None
            }
        };
        // A stream that was stopped early isn't complete; it starts over next time.
        let Some((_, stream)) = finished else { return };
        let signed_in = stream.signed_in;
        stream.stop();
        let here = *self.view.borrow() == View::Channel(id.to_string());
        let Some(reason) = failure else {
            self.history_done.borrow_mut().insert(id.to_string());
            if here {
                if self.shown.borrow().is_empty() {
                    self.note.set_label("This channel has no videos to show.");
                }
                self.update_hero(false);
            }
            return;
        };
        // A failed read isn't marked done, so the next visit tries again.
        if !here {
            return;
        }
        if !signed_in && util::is_bot_check(&reason) && tools::sign_in().is_some() {
            self.start_history(id, true);
            return;
        }
        if self.shown.borrow().is_empty() {
            self.note.set_label("This channel's videos didn't load.");
        }
        self.update_hero(false);
        let sign_in = tools::sign_in().filter(|_| signed_in);
        self.toast_refusal(&reason, sign_in.as_ref(), "Older videos from this channel didn't load");
    }

    /// Near the end of a channel's list: read the next page of older uploads.
    fn want_more_history(&self) {
        let View::Channel(id) = self.view.borrow().clone() else { return };
        let streams = self.history_stream.borrow();
        if let Some((running, stream)) = streams.as_ref() {
            if *running == id && !self.history_asked.replace(true) {
                stream.more();
            }
        }
    }

    fn stop_history(&self) {
        let running = self.history_stream.borrow_mut().take();
        if let Some((_, stream)) = running {
            stream.stop();
        }
    }

    // ── Grid ───────────────────────────────────────────────────────────

    fn card_of(&self, item: &gtk::ListItem) -> Option<Rc<Card>> {
        let root = item.child().and_downcast::<gtk::Button>()?;
        self.cards.borrow().get(&root).cloned()
    }

    /// How many cards fit across, the way the grid works it out.
    fn columns(&self) -> usize {
        let width = self.scroller.width() - 2 * (SIDE_PADDING - SPACING / 2);
        (width / self.card_min.get()).clamp(1, 8) as usize
    }

    fn bind_card(self: &Rc<Self>, card: &Rc<Card>, video: &Video, position: u32) {
        let _timing = perf::span("card", 2);
        self.release_card(card);
        let opts = {
            let st = self.store.borrow();
            CardOpts {
                is_new: self.new_ids.borrow().contains(&video.id),
                watched: st.watched.contains(&video.id),
                show_channel: !matches!(*self.view.borrow(), View::Channel(_)),
                subscribed: st.is_subscribed(&video.channel_id),
                progress: st.progress.get(&video.id).map_or(0.0, |&[position, duration]| {
                    if duration > 0.0 {
                        position / duration
                    } else {
                        0.0
                    }
                }),
            }
        };
        card.bind(video, opts, util::now());
        card.set_position(position);
        self.bound.borrow_mut().insert(video.id.clone(), Rc::downgrade(card));
        // The image is fetched once the card is near the screen.
        card.want_thumb(&video.id);

        let columns = self.columns();
        let position = position as usize;
        if glib::monotonic_time() < self.entrance_until.get() && position < columns * 4 {
            // A diagonal wave from the top-left corner.
            card.enter_after(30 + 60 * (position / columns + position % columns) as u64);
        }
        // Keyboard focus brings the card into view.
        let grid = self.grid.downgrade();
        card.set_on_focus(move || {
            if let Some(grid) = grid.upgrade() {
                grid.scroll_to(position as u32, gtk::ListScrollFlags::NONE, None);
            }
        });
        self.queue_visibility();
    }

    fn release_card(&self, card: &Rc<Card>) {
        if let Some((id, token)) = card.take_thumb_request() {
            self.thumbs.cancel(&id, token);
        }
        if let Some(id) = card.bound_id() {
            let mut bound = self.bound.borrow_mut();
            let same = bound
                .get(&id)
                .and_then(Weak::upgrade)
                .is_some_and(|other| Rc::ptr_eq(&other, card));
            if same {
                bound.remove(&id);
            }
        }
        card.unbind();
    }

    fn bound_card(&self, id: &str) -> Option<Rc<Card>> {
        self.bound.borrow().get(id).and_then(Weak::upgrade)
    }

    /// Runs the thumbnail pass soon, at most every few frames while scrolling.
    fn queue_visibility(self: &Rc<Self>) {
        if self.visibility_queued.replace(true) {
            return;
        }
        let wait = (self.last_visibility.get() + VISIBILITY_INTERVAL_US - glib::monotonic_time()).max(0);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_micros(wait as u64), move || {
            if let Some(win) = weak.upgrade() {
                win.visibility_queued.set(false);
                win.last_visibility.set(glib::monotonic_time());
                win.update_visibility();
            }
        });
    }

    /// Fetches thumbnails for the cards on screen plus a screenful ahead and
    /// behind, nearest first, and lets go of images far away so memory stays
    /// flat on long scrolls. The grid only positions on-screen cards, so
    /// "ahead" is counted in places in the feed rather than in pixels.
    fn update_visibility(&self) {
        let _timing = perf::span("thumbnail pass", 2);
        let height = self.scroller.height() as f32;
        if height <= 0.0 {
            return;
        }
        let cards: Vec<Rc<Card>> = self
            .cards
            .borrow()
            .values()
            .filter(|card| card.bound_id().is_some())
            .cloned()
            .collect();
        let (mut first, mut last) = (u32::MAX, 0);
        for card in &cards {
            let Some(bounds) = card.root.compute_bounds(&self.scroller) else { continue };
            let on_screen = bounds.width() > 0.0
                && bounds.height() > 0.0
                && bounds.y() + bounds.height() >= 0.0
                && bounds.y() <= height;
            if on_screen {
                first = first.min(card.position());
                last = last.max(card.position());
            }
        }
        if first == u32::MAX {
            return;
        }
        let screen = last - first + 1;
        if last + 2 * screen >= self.model.n_items() && self.grid_queue.borrow().is_empty() {
            self.want_more_history();
        }
        let near = first.saturating_sub(screen)..=last + screen;
        let kept = first.saturating_sub(3 * screen)..=last + 3 * screen;
        let mut wanted: Vec<(u32, Rc<Card>)> = Vec::new();
        for card in cards {
            let position = card.position();
            if near.contains(&position) {
                if card.has_wanted_thumb() {
                    // On screen in reading order first, then outward.
                    let order = if position < first {
                        screen + (first - position)
                    } else if position > last {
                        screen + (position - last)
                    } else {
                        position - first
                    };
                    wanted.push((order, card));
                }
            } else if !kept.contains(&position) {
                if let Some((id, token)) = card.forget_thumb() {
                    self.thumbs.cancel(&id, token);
                }
            }
        }
        // The queue serves the newest request first, so ask for the farthest first.
        wanted.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, card) in wanted {
            self.request_thumb(&card);
        }
    }

    fn request_thumb(&self, card: &Rc<Card>) {
        let Some(id) = card.take_wanted_thumb() else { return };
        // Images already in memory appear at once; fetched ones fade up.
        let weak = Rc::downgrade(card);
        let video_id = id.clone();
        let waiting = Rc::new(Cell::new(false));
        let fade = waiting.clone();
        let token = self.thumbs.get(&id, move |texture| {
            if let Some(card) = weak.upgrade() {
                card.show_texture(&video_id, texture, fade.get());
            }
        });
        waiting.set(true);
        card.set_thumb_request(token.map(|token| (id, token)));
    }

    fn update_hero(&self, animate: bool) {
        let view = self.view.borrow().clone();
        let shown = self.shown.borrow().len();
        let (title, sub, news) = {
            let st = self.store.borrow();
            match &view {
                View::Feed => {
                    let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
                    let new_count = {
                        let new_ids = self.new_ids.borrow();
                        st.feed
                            .iter()
                            .filter(|v| new_ids.contains(&v.id) && !(st.hide_shorts && v.is_short))
                            .count()
                    };
                    let sub = if st.channels.is_empty() {
                        String::new()
                    } else if new_count > 0 {
                        format!("{} since your last visit", util::count(new_count, "new video", "new videos"))
                    } else {
                        format!("The latest from {}", util::count(st.channels.len(), "channel", "channels"))
                    };
                    (util::greeting(hour).to_string(), sub, new_count > 0)
                }
                View::Trending => (
                    "Trending".to_string(),
                    "Uploads doing unusually well for their channel lately".to_string(),
                    false,
                ),
                View::Channel(id) => {
                    let sub = if self.history_running(id) {
                        format!("{} so far; older ones load as you scroll", util::count(shown, "video", "videos"))
                    } else {
                        util::count(shown, "video", "videos")
                    };
                    (st.channel_name(id).unwrap_or_default(), sub, false)
                }
                View::Search(query) => (format!("“{query}”"), util::count(shown, "result", "results"), false),
            }
        };

        let is_channel = matches!(view, View::Channel(_));
        self.hero_avatar.set_visible(is_channel);
        if is_channel {
            self.hero_avatar.set_text(Some(&title));
        }
        self.hero_action.set_visible(is_channel);
        self.filters.set_visible(!matches!(view, View::Search(_)));
        self.hero_title.set_label(&title);
        self.hero_sub.set_label(&sub);
        self.hero_sub.set_visible(!sub.is_empty());
        if news {
            self.hero_sub.add_css_class("news");
        } else {
            self.hero_sub.remove_css_class("news");
        }
        if animate {
            let flip = !self.hero_flip.get();
            self.hero_flip.set(flip);
            let (on, off) = if flip { ("rise-a", "rise-b") } else { ("rise-b", "rise-a") };
            self.hero.remove_css_class(off);
            self.hero.add_css_class(on);
        }
    }

    fn show_empty(&self, view: &View) {
        let (no_channels, hide_shorts, feed_empty) = {
            let st = self.store.borrow();
            (st.channels.is_empty(), st.hide_shorts, st.feed.is_empty())
        };
        let shorts_hint = "Uncheck Hide Shorts in the main menu to see them.";
        let (icon, title, description, welcome) = match view {
            View::Search(query) => match self.search_error.borrow().clone() {
                Some(error) => ("system-search-symbolic", "Search didn't work", error, false),
                None => (
                    "system-search-symbolic",
                    "No videos found",
                    format!("Nothing matched “{query}”. Try different words."),
                    false,
                ),
            },
            View::Channel(_) if hide_shorts => (
                "video-x-generic-symbolic",
                "Nothing here",
                format!("This channel's uploads are all Shorts. {shorts_hint}"),
                false,
            ),
            View::Channel(_) => (
                "video-x-generic-symbolic",
                "Nothing here",
                "This channel hasn't uploaded any videos.".to_string(),
                false,
            ),
            View::Feed | View::Trending if no_channels => (
                "video-x-generic-symbolic",
                "Build your feed",
                "Bring your subscriptions over from Google Takeout, NewPipe or FreeTube, \
                 or add channels one at a time. Matinee never signs in to a Google account."
                    .to_string(),
                true,
            ),
            View::Feed | View::Trending if feed_empty => (
                "network-offline-symbolic",
                "No videos yet",
                "Your channels couldn't be loaded. Check your connection, then refresh.".to_string(),
                false,
            ),
            View::Feed | View::Trending => (
                "video-x-generic-symbolic",
                "Nothing to show",
                format!("Every recent upload is a Short. {shorts_hint}"),
                false,
            ),
        };
        match logo_texture().filter(|_| welcome) {
            Some(logo) => self.empty.set_paintable(Some(&logo)),
            None => {
                self.empty.set_paintable(None::<&gdk::Paintable>);
                self.empty.set_icon_name(Some(&icons::get(icon)));
            }
        }
        self.empty.set_title(title);
        // The description is markup; queries and errors are not.
        self.empty
            .set_description(Some(&glib::markup_escape_text(&description)));
        self.empty_actions.set_visible(welcome);
        self.hero.set_visible(false);
        self.stack.set_visible_child_name("empty");
    }

    fn scroll_to_top(&self) {
        let adjustment = self.scroller.vadjustment();
        // From far down, jump most of the way first: gliding past hundreds of
        // rows would build every one of them on the way.
        let near = adjustment.page_size() * 2.0;
        if adjustment.value() > near {
            adjustment.set_value(near);
        }
        let target = adw::PropertyAnimationTarget::new(&adjustment, "value");
        let animation = adw::TimedAnimation::new(&self.scroller, adjustment.value(), 0.0, 520, target);
        animation.set_easing(adw::Easing::EaseOutCubic);
        animation.play();
        self.scroll_animation.replace(Some(animation));
    }

    fn show_new_pill(&self) {
        let count = {
            let st = self.store.borrow();
            let new_ids = self.new_ids.borrow();
            let shown = self.shown.borrow();
            let on_screen: HashSet<&str> = shown.iter().map(|v| v.id.as_str()).collect();
            st.feed
                .iter()
                .filter(|v| {
                    new_ids.contains(&v.id) && !on_screen.contains(v.id.as_str()) && !(st.hide_shorts && v.is_short)
                })
                .count()
        };
        if count > 0 {
            self.new_pill_button
                .set_label(&format!("Show {}", util::count(count, "new video", "new videos")));
            self.new_pill.set_reveal_child(true);
        }
        self.update_hero(false);
    }

    // ── Refreshing ─────────────────────────────────────────────────────

    fn refresh(self: &Rc<Self>, manual: bool) {
        let channels = self.store.borrow().channels.clone();
        if channels.is_empty() {
            if manual {
                self.render();
            }
            return;
        }
        self.fetch(channels, Fetch::Refresh { manual });
    }

    fn fetch(self: &Rc<Self>, channels: Vec<Channel>, mode: Fetch) {
        if let Fetch::Refresh { .. } = mode {
            if self.refreshing.replace(true) {
                return;
            }
        }
        self.busy.set(self.busy.get() + 1);
        self.refresh_button.add_css_class("spinning");
        self.refresh_button.set_tooltip_text(Some("Refreshing…"));

        let first_load = self.store.borrow().feed.is_empty();
        // A first load (or a new channel) is fetched quickly while you wait;
        // later checks take their time, so YouTube never sees a burst.
        let gentle = matches!(mode, Fetch::Refresh { .. }) && !first_load;
        if first_load && !self.in_search() {
            self.loading_title.set_label("Fetching your feed");
            self.loading_sub.set_label(&format!(
                "Checking {} for new uploads",
                util::count(channels.len(), "channel", "channels")
            ));
            self.hero.set_visible(false);
            self.stack.set_visible_child_name("loading");
        }

        let this = self.clone();
        glib::spawn_future_local(async move {
            let report = gio::spawn_blocking(move || feed::fetch_all(&channels, gentle)).await;
            if let Fetch::Refresh { .. } = mode {
                this.refreshing.set(false);
                this.store.borrow_mut().last_refresh = Some(util::now());
            }
            this.busy.set(this.busy.get() - 1);
            if this.busy.get() == 0 {
                this.refresh_button.remove_css_class("spinning");
                this.refresh_button.set_tooltip_text(Some("Refresh"));
            }
            let Ok(report) = report else {
                this.toast("Refresh stopped unexpectedly. Try again.");
                return;
            };
            let user_asked = match mode {
                Fetch::Refresh { manual } => manual || first_load,
                Fetch::Added => true,
            };
            this.apply_report(report, user_asked);
        });
    }

    fn apply_report(self: &Rc<Self>, report: feed::Report, user_asked: bool) {
        let first_run = self.store.borrow().seen.is_empty();
        let (fresh, renamed) = self.merge_report(&report);
        let mut arrived = 0;
        {
            let mut quiet = self.quiet_channels.borrow_mut();
            let mut st = self.store.borrow_mut();
            let mut new_ids = self.new_ids.borrow_mut();
            for (id, channel_id) in fresh {
                if first_run || quiet.contains(&channel_id) || st.watched.contains(&id) {
                    st.seen.insert(id);
                } else if new_ids.insert(id) {
                    arrived += 1;
                }
            }
            for f in &report.feeds {
                quiet.remove(&f.channel.id);
            }
        }
        self.save_later();
        if renamed {
            self.rebuild_sidebar();
        } else {
            self.update_sidebar_counts();
        }

        let failed = report.failed.len();
        let total = failed + report.feeds.len();
        if failed > 0 && (user_asked || failed == total) {
            let message = if failed == total {
                "Couldn't reach YouTube. Check your connection, then refresh.".to_string()
            } else if failed == 1 {
                format!("Couldn't refresh {}", report.failed[0].0.name)
            } else {
                format!("{failed} channels didn't refresh; their older videos are still here")
            };
            self.toast(&message);
        }

        if self.in_search() {
            return;
        }
        if user_asked {
            self.render();
        } else if arrived > 0 {
            // Don't yank the feed out from under someone; offer the new videos instead.
            self.show_new_pill();
        } else {
            self.update_hero(false);
        }
    }

    /// Folds fetched channels into the stored feed. Returns (video, channel)
    /// pairs for uploads that haven't been on screen yet, and whether any
    /// channel changed its name.
    fn merge_report(&self, report: &feed::Report) -> (Vec<(String, String)>, bool) {
        let mut st = self.store.borrow_mut();
        let fetched: HashSet<&str> = report.feeds.iter().map(|f| f.channel.id.as_str()).collect();
        let subscribed: HashSet<String> = st.channels.iter().map(|c| c.id.clone()).collect();
        let mut renamed = false;

        // Channels that failed keep their previous videos.
        let mut feed: Vec<Video> = st
            .feed
            .iter()
            .filter(|v| !fetched.contains(v.channel_id.as_str()))
            .cloned()
            .collect();
        for f in &report.feeds {
            if let Some(c) = st.channels.iter_mut().find(|c| c.id == f.channel.id) {
                if !f.channel.name.is_empty() && c.name != f.channel.name {
                    c.name = f.channel.name.clone();
                    renamed = true;
                }
            }
            feed.extend(f.videos.iter().cloned());
        }
        feed.retain(|v| subscribed.contains(&v.channel_id));
        feed.sort_by(|a, b| b.published.cmp(&a.published));
        let mut unique = HashSet::new();
        feed.retain(|v| unique.insert(v.id.clone()));

        st.forget_leaving(&feed);
        let fresh = feed
            .iter()
            .filter(|v| !st.seen.contains(&v.id))
            .map(|v| (v.id.clone(), v.channel_id.clone()))
            .collect();
        st.feed = feed;
        (fresh, renamed)
    }

    // ── Search ─────────────────────────────────────────────────────────

    fn start_search(self: &Rc<Self>, query: String) {
        let query = query.trim().to_string();
        if query.is_empty() {
            if self.in_search() {
                self.show_view(View::Feed);
            }
            return;
        }
        let generation = self.search_gen.get() + 1;
        self.search_gen.set(generation);
        self.stop_history();
        *self.view.borrow_mut() = View::Search(query.clone());
        *self.search_error.borrow_mut() = None;
        self.select_row_for_view();
        self.new_pill.set_reveal_child(false);
        self.loading_title.set_label("Searching");
        self.loading_sub.set_label(&format!("“{query}”"));
        self.hero.set_visible(false);
        self.stack.set_visible_child_name("loading");
        self.after_navigation();
        self.stop_search();
        self.results.borrow_mut().clear();
        self.run_search(query, generation, false);
    }

    /// Runs the search, showing results as they arrive. The browser sign-in
    /// is used only after YouTube refused to answer without it.
    fn run_search(self: &Rc<Self>, query: String, generation: u64, signed_in: bool) {
        let (stream, updates) = match search::start(&query, SEARCH_RESULTS, signed_in) {
            Ok(started) => started,
            Err(message) => {
                *self.search_error.borrow_mut() = Some(message);
                self.render();
                return;
            }
        };
        *self.search_stream.borrow_mut() = Some(stream);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let mut failure = None;
            while let Ok(update) = updates.recv().await {
                // A search that has since been replaced is ignored.
                if this.search_gen.get() != generation || !this.in_search() {
                    return;
                }
                match update {
                    ytdlp::Update::Videos(batch) => this.add_results(batch),
                    ytdlp::Update::Failed(reason) => failure = Some(reason),
                }
            }
            if this.search_gen.get() == generation && this.in_search() {
                this.finish_search(query, generation, signed_in, failure);
            }
        });
    }

    fn add_results(self: &Rc<Self>, batch: Vec<Video>) {
        let (first, fresh) = {
            let mut results = self.results.borrow_mut();
            let mut known: HashSet<String> = results.iter().map(|v| v.id.clone()).collect();
            let fresh: Vec<Video> = batch.into_iter().filter(|v| known.insert(v.id.clone())).collect();
            let first = results.is_empty();
            results.extend(fresh.iter().cloned());
            (first, fresh)
        };
        if first {
            // The first results replace the "Searching" page, with the entrance.
            self.render();
            return;
        }
        let now = util::now();
        let visible: Vec<Video> = {
            let st = self.store.borrow();
            fresh.into_iter().filter(|v| self.passes_filter(v, &st, now)).collect()
        };
        if !visible.is_empty() {
            let items: Vec<glib::BoxedAnyObject> = visible.iter().cloned().map(glib::BoxedAnyObject::new).collect();
            self.append_grid(items);
            self.shown.borrow_mut().extend(visible);
        }
        self.update_hero(false);
    }

    fn finish_search(self: &Rc<Self>, query: String, generation: u64, signed_in: bool, failure: Option<String>) {
        self.stop_search();
        let nothing = self.results.borrow().is_empty();
        let Some(reason) = failure else {
            if nothing {
                // "No videos found".
                self.render();
            }
            return;
        };
        let sign_in = tools::sign_in();
        if !nothing {
            self.toast_refusal(&reason, sign_in.filter(|_| signed_in).as_ref(), "Some results didn't load");
            return;
        }
        if !signed_in && sign_in.is_some() && util::is_bot_check(&reason) {
            self.run_search(query, generation, true);
            return;
        }
        let message = match util::explain_ytdlp(&reason, sign_in.filter(|_| signed_in).as_ref()) {
            Some(explained) => format!("{}. {}", explained.title, explained.detail),
            None => format!("yt-dlp couldn't search: {reason}"),
        };
        *self.search_error.borrow_mut() = Some(message);
        self.render();
    }

    fn stop_search(&self) {
        if let Some(stream) = self.search_stream.borrow_mut().take() {
            stream.stop();
        }
    }

    // ── Subscriptions ──────────────────────────────────────────────────

    /// Adds channels that aren't already subscribed and fetches their videos.
    /// Returns the ones that were actually added.
    fn add_channels(self: &Rc<Self>, channels: Vec<Channel>) -> Vec<Channel> {
        let added: Vec<Channel> = {
            let mut st = self.store.borrow_mut();
            let mut added: Vec<Channel> = Vec::new();
            for c in channels {
                if !st.is_subscribed(&c.id) && !added.iter().any(|a| a.id == c.id) {
                    added.push(c);
                }
            }
            st.channels.extend(added.iter().cloned());
            added
        };
        if added.is_empty() {
            return added;
        }
        self.save_later();
        self.quiet_channels
            .borrow_mut()
            .extend(added.iter().map(|c| c.id.clone()));
        let bound: Vec<Rc<Card>> = self.bound.borrow().values().filter_map(Weak::upgrade).collect();
        for card in bound {
            let id = card.bound_id().unwrap_or_default();
            let channel = self.find_video(&id).map(|v| v.channel_id).unwrap_or_default();
            if added.iter().any(|c| c.id == channel) {
                card.set_subscribed(true);
            }
        }
        self.rebuild_sidebar();
        self.fetch(added.clone(), Fetch::Added);
        added
    }

    fn subscribe_to_channel_of(self: &Rc<Self>, video_id: &str) {
        let Some(video) = self.find_video(video_id) else { return };
        if video.channel_id.is_empty() {
            self.toast("Couldn't tell which channel posted this video");
            return;
        }
        let name = if video.channel_name.is_empty() {
            video.channel_id.clone()
        } else {
            video.channel_name.clone()
        };
        let channel = Channel { id: video.channel_id, name: name.clone() };
        if self.add_channels(vec![channel]).is_empty() {
            self.toast(&format!("Already subscribed to {name}"));
        } else {
            self.toast(&format!("Subscribed to {name}"));
        }
    }

    fn add_channel_dialog(self: &Rc<Self>) {
        let entry = gtk::Entry::builder()
            .placeholder_text("youtube.com/@channel")
            .activates_default(true)
            .build();
        entry.connect_map(|entry| {
            entry.grab_focus();
        });
        let dialog = adw::AlertDialog::new(
            Some("Add a channel"),
            Some("Paste a channel link, @handle or channel ID. A link to one of its videos works too."),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("add", "Add")]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("add"));
        dialog.set_close_response("cancel");
        dialog.set_response_enabled("add", false);
        dialog.set_extra_child(Some(&entry));
        let weak_dialog = dialog.downgrade();
        entry.connect_changed(move |entry| {
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.set_response_enabled("add", !entry.text().trim().is_empty());
            }
        });

        let this = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&this.window)).await.as_str() != "add" {
                return;
            }
            let input = entry.text().to_string();
            let pending = this.toast("Looking up channel…");
            pending.set_timeout(0);
            let result = gio::spawn_blocking(move || resolve::resolve(&input)).await;
            pending.dismiss();
            match result {
                Ok(Ok(channel)) => {
                    let name = channel.name.clone();
                    if this.add_channels(vec![channel]).is_empty() {
                        this.toast(&format!("Already subscribed to {name}"));
                    } else {
                        this.toast(&format!("Subscribed to {name}"));
                    }
                }
                Ok(Err(message)) => {
                    this.toast(&message);
                }
                Err(_) => {}
            }
        });
    }

    fn import_dialog(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Subscription exports"));
        for pattern in ["*.csv", "*.json", "*.db", "*.opml", "*.xml", "*.txt"] {
            filter.add_pattern(pattern);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Import subscriptions")
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();

        let this = self.clone();
        glib::spawn_future_local(async move {
            let Ok(file) = dialog.open_future(Some(&this.window)).await else { return };
            let bytes = match file.load_contents_future().await {
                Ok((bytes, _)) => bytes,
                Err(e) => {
                    this.toast(&format!("Couldn't read that file: {}", e.message()));
                    return;
                }
            };
            let channels = resolve::parse_import(&String::from_utf8_lossy(&bytes));
            if channels.is_empty() {
                this.toast("No channels in that file. In a Google Takeout export, choose subscriptions.csv.");
                return;
            }
            let found = channels.len();
            let added = this.add_channels(channels).len();
            let message = match (added, found - added) {
                (0, _) => "You're already subscribed to every channel in that file".to_string(),
                (n, 0) => format!("Imported {}", util::count(n, "channel", "channels")),
                (n, skipped) => format!("Imported {}; {skipped} were already here", util::count(n, "channel", "channels")),
            };
            this.toast(&message);
        });
    }

    fn unsubscribe_current(self: &Rc<Self>) {
        let View::Channel(id) = self.view.borrow().clone() else { return };
        let name = self.store.borrow().channel_name(&id).unwrap_or_default();
        let dialog = adw::AlertDialog::new(
            Some(&format!("Unsubscribe from {name}?")),
            Some("Their videos leave your feed. You can subscribe again anytime."),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("unsubscribe", "Unsubscribe")]);
        dialog.set_response_appearance("unsubscribe", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let this = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&this.window)).await.as_str() != "unsubscribe" {
                return;
            }
            this.stop_history();
            this.history.borrow_mut().remove(&id);
            this.history_done.borrow_mut().remove(&id);
            {
                let mut st = this.store.borrow_mut();
                st.channels.retain(|c| c.id != id);
                st.feed.retain(|v| v.channel_id != id);
            }
            this.save_later();
            *this.view.borrow_mut() = View::Feed;
            this.rebuild_sidebar();
            this.render();
            this.toast(&format!("Unsubscribed from {name}"));
        });
    }

    // ── Videos ─────────────────────────────────────────────────────────

    fn find_video(&self, id: &str) -> Option<Video> {
        let find = |list: &[Video]| list.iter().find(|v| v.id == id).cloned();
        find(&self.shown.borrow())
            .or_else(|| find(&self.results.borrow()))
            .or_else(|| find(&self.store.borrow().feed))
    }

    fn play(self: &Rc<Self>, id: &str) {
        let Some(video) = self.find_video(id) else { return };
        if self.store.borrow().external_player {
            self.play_external(&video, None);
            return;
        }
        let resume = {
            let st = self.store.borrow();
            if st.no_resume {
                None
            } else {
                st.progress.get(id).map(|&[position, _]| position)
            }
        };
        // In picture in picture the new video plays in the small window.
        let watching = self.nav.visible_page().and_then(|p| p.tag()).as_deref() == Some("watch");
        if !watching && !self.watch.in_pip() {
            self.nav.push_by_tag("watch");
        }
        self.watch.open(&video, resume);
        let watch = Rc::downgrade(&self.watch);
        let video_id = video.id.clone();
        self.thumbs.get(&video.id, move |texture| {
            if let Some(watch) = watch.upgrade() {
                watch.set_poster(&video_id, texture);
            }
        });
    }

    /// Plays in mpv's own window, optionally from `start` seconds in.
    fn play_external(self: &Rc<Self>, video: &Video, start: Option<f64>) {
        let id = video.id.as_str();
        let quality = self.store.borrow().quality;
        let height = quality.unwrap_or_else(|| watch::screen_height(Some(self.window.upcast_ref())));
        match player::spawn(video, start, height) {
            Ok(child) => {
                self.toast(&format!(
                    "Playing “{}” in {}",
                    util::ellipsize(&video.title, 48),
                    player::name()
                ));
                let was_watched = self.store.borrow().watched.contains(id);
                self.set_watched(id, true);
                // Wait on a plain thread: a long film shouldn't tie up the I/O pool.
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(player::wait(child));
                });
                let this = self.clone();
                let id = id.to_string();
                glib::spawn_future_local(async move {
                    if let Ok(Err(reason)) = rx.recv().await {
                        // yt-dlp refused before anything played: not watched after all.
                        if !reason.is_empty() && !was_watched {
                            this.set_watched(&id, false);
                        }
                        this.toast_refusal(
                            &reason,
                            tools::sign_in().as_ref(),
                            "mpv couldn't play that video. Updating yt-dlp usually fixes this.",
                        );
                    }
                });
            }
            Err(message) => {
                self.toast(&message);
            }
        }
    }

    fn set_watched(&self, id: &str, watched: bool) {
        let changed = {
            let mut st = self.store.borrow_mut();
            if watched {
                st.watched.insert(id.to_string())
            } else {
                st.watched.remove(id)
            }
        };
        if changed {
            self.save_later();
        }
        let was_new = watched && self.new_ids.borrow_mut().remove(id);
        if let Some(card) = self.bound_card(id) {
            card.set_watched(watched);
        }
        if was_new {
            self.update_sidebar_counts();
            self.update_hero(false);
        }
    }

    /// Remembers where an unfinished video was left, so it can resume there:
    /// anything past the first seconds and short of the end.
    fn record_progress(&self, video: &Video, position: f64, duration: f64) {
        let unfinished = duration > 0.0 && position > 5.0 && duration - position > 20.0 && position / duration < 0.95;
        {
            let mut st = self.store.borrow_mut();
            if unfinished {
                st.progress.insert(video.id.clone(), [position, duration]);
            } else {
                st.progress.remove(&video.id);
            }
        }
        self.save_later();
        if let Some(card) = self.bound_card(&video.id) {
            card.set_progress(if unfinished { position / duration } else { 0.0 });
        }
    }

    fn copy_link(&self, id: &str) {
        if let Some(video) = self.find_video(id) {
            self.window.clipboard().set_text(&video.watch_url());
            self.toast("Link copied");
        }
    }

    fn open_in_browser(&self, id: &str) {
        let Some(video) = self.find_video(id) else { return };
        if let Err(e) = gio::AppInfo::launch_default_for_uri(&video.watch_url(), None::<&gio::AppLaunchContext>) {
            self.toast(&format!("Couldn't open a browser: {}", e.message()));
        }
    }

    // ── Preferences ────────────────────────────────────────────────────

    fn set_card_size(&self, index: u8) {
        let index = index.min(2);
        self.store.borrow_mut().card_size = Some(index);
        self.save_later();
        let min = CARD_SIZES[usize::from(index)];
        self.card_min.set(min);
        for card in self.cards.borrow().values() {
            card.set_min_width(min - SPACING);
        }
        self.grid.set_max_columns(self.columns() as u32);
        self.grid.queue_resize();
    }

    fn preferences(self: &Rc<Self>) {
        let (always_dark, card_size, hide_shorts, refresh_minutes, quality, resume, external, smooth_scrolling) = {
            let st = self.store.borrow();
            (
                st.always_dark,
                st.card_size.unwrap_or(1),
                st.hide_shorts,
                st.refresh_minutes.unwrap_or(DEFAULT_REFRESH_MINUTES),
                st.quality,
                !st.no_resume,
                st.external_player,
                !st.no_smooth_scroll,
            )
        };
        let page = adw::PreferencesPage::new();

        let appearance = adw::PreferencesGroup::builder().title("Appearance").build();
        let dark = switch_row("Always use dark style", "Ignore the system's light or dark setting", always_dark);
        let size = combo_row("Card size", &["Small", "Medium", "Large"], usize::from(card_size));
        appearance.add(&dark);
        appearance.add(&size);

        let feed = adw::PreferencesGroup::builder().title("Feed").build();
        let shorts = switch_row("Hide Shorts", "Leave Shorts out of the feed, channels and search", hide_shorts);
        let refresh_labels: Vec<&str> = REFRESH_CHOICES.iter().map(|(_, label)| *label).collect();
        let refresh_index = REFRESH_CHOICES.iter().position(|(m, _)| *m == refresh_minutes).unwrap_or(1);
        let refresh = combo_row("Check for new videos", &refresh_labels, refresh_index);
        feed.add(&shorts);
        feed.add(&refresh);

        let playback = adw::PreferencesGroup::builder().title("Playback").build();
        let quality_labels: Vec<&str> = QUALITIES.iter().map(|(_, label)| *label).collect();
        let quality_index = QUALITIES.iter().position(|(q, _)| *q == quality).unwrap_or(0);
        let quality_row = combo_row("Video quality", &quality_labels, quality_index);
        quality_row.set_subtitle("Where a video starts; change it for one video from the player");
        let resume_row = switch_row("Resume where I left off", "", resume);
        let external_row = switch_row("Play in a separate window", "Open videos in mpv's own window", external);
        let sign_in = self.store.borrow().cookies_browser.clone();
        let choices = self.sign_in_choices();
        let browser_labels: Vec<&str> = choices.iter().map(|(_, name)| name.as_str()).collect();
        let browser_index = choices.iter().position(|(key, _)| *key == sign_in).unwrap_or(0);
        let sign_in_row = combo_row("Use a browser's YouTube sign-in", &browser_labels, browser_index);
        sign_in_row.set_subtitle(
            "Only if YouTube keeps asking whether you're a bot. YouTube can then link what you watch \
             to that account. Firefox works most reliably.",
        );
        playback.add(&quality_row);
        playback.add(&resume_row);
        playback.add(&external_row);
        playback.add(&sign_in_row);

        let browsing = adw::PreferencesGroup::builder().title("Browsing").build();
        let smooth_row = switch_row("Smooth wheel scrolling", "Glide with each wheel notch instead of jumping", smooth_scrolling);
        let sidebar_row = switch_row("Show the channel list", "Also F9 or the button at the top left", self.split.shows_sidebar());
        browsing.add(&smooth_row);
        browsing.add(&sidebar_row);

        for group in [&appearance, &feed, &playback, &browsing] {
            page.add(group);
        }

        let this = self.clone();
        dark.connect_active_notify(move |row| this.set_toggle("always-dark", row.is_active()));
        let this = self.clone();
        size.connect_selected_notify(move |row| this.set_card_size(row.selected() as u8));
        let this = self.clone();
        shorts.connect_active_notify(move |row| this.set_toggle("hide-shorts", row.is_active()));
        let this = self.clone();
        refresh.connect_selected_notify(move |row| {
            let minutes = REFRESH_CHOICES[(row.selected() as usize).min(REFRESH_CHOICES.len() - 1)].0;
            this.store.borrow_mut().refresh_minutes = Some(minutes);
            this.save_later();
        });
        let this = self.clone();
        quality_row.connect_selected_notify(move |row| {
            let quality = QUALITIES[(row.selected() as usize).min(QUALITIES.len() - 1)].0;
            this.store.borrow_mut().quality = quality;
            this.save_later();
            this.watch.set_preferred_quality(quality);
        });
        let this = self.clone();
        resume_row.connect_active_notify(move |row| {
            this.store.borrow_mut().no_resume = !row.is_active();
            this.save_later();
        });
        let this = self.clone();
        external_row.connect_active_notify(move |row| this.set_toggle("external-player", row.is_active()));
        let this = self.clone();
        sign_in_row.connect_selected_notify(move |row| {
            let key = choices.get(row.selected() as usize).and_then(|(key, _)| key.clone());
            this.use_sign_in(key);
        });
        let this = self.clone();
        smooth_row.connect_active_notify(move |row| {
            this.store.borrow_mut().no_smooth_scroll = !row.is_active();
            this.save_later();
            smooth::set_enabled(row.is_active());
        });
        let this = self.clone();
        sidebar_row.connect_active_notify(move |row| this.split.set_show_sidebar(row.is_active()));

        let dialog = adw::PreferencesDialog::new();
        dialog.set_title("Preferences");
        dialog.add(&page);
        dialog.present(Some(&self.window));
    }

    // ── Misc ───────────────────────────────────────────────────────────

    fn toast(&self, message: &str) -> adw::Toast {
        let toast = adw::Toast::new(message);
        toast.set_use_markup(false);
        toast.set_timeout(4);
        self.toasts.add_toast(toast.clone());
        toast
    }

    /// Says what YouTube refused, in plain words, with the way to a browser
    /// sign-in; `otherwise` for any other failure.
    fn toast_refusal(&self, reason: &str, sign_in: Option<&tools::Browser>, otherwise: &str) {
        let Some(explained) = util::explain_ytdlp(reason, sign_in) else {
            self.toast(otherwise);
            return;
        };
        let toast = self.toast(&explained.title);
        toast.set_button_label(Some("Choose a browser…"));
        toast.set_action_name(Some("win.sign-in"));
        toast.set_timeout(10);
    }

    /// The choices for the browser sign-in: off, the browsers set up here,
    /// and the chosen one even if it has since gone.
    fn sign_in_choices(&self) -> Vec<(Option<String>, String)> {
        let chosen = self.store.borrow().cookies_browser.clone();
        let mut browsers = tools::browsers();
        if let Some(key) = chosen.as_deref() {
            if !browsers.iter().any(|browser| browser.key == key) {
                browsers.push(tools::browser(key));
            }
        }
        std::iter::once((None, "Off".to_string()))
            .chain(browsers.into_iter().map(|browser| (Some(browser.key), browser.name)))
            .collect()
    }

    /// Asks which browser's YouTube sign-in to borrow; reached from YouTube's
    /// refusals. Preferences has the same setting.
    fn sign_in_dialog(self: &Rc<Self>) {
        let current = self.store.borrow().cookies_browser.clone();
        let chosen = Rc::new(RefCell::new(current.clone()));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let mut group: Option<gtk::CheckButton> = None;
        for (key, name) in self.sign_in_choices() {
            let check = gtk::CheckButton::new();
            check.set_group(group.as_ref());
            check.set_active(key == current);
            let picked = chosen.clone();
            check.connect_toggled(move |check| {
                if check.is_active() {
                    *picked.borrow_mut() = key.clone();
                }
            });
            let row = adw::ActionRow::builder()
                .title(name)
                .use_markup(false)
                .activatable_widget(&check)
                .build();
            row.add_prefix(&check);
            list.append(&row);
            group.get_or_insert(check);
        }
        let dialog = adw::AlertDialog::new(
            Some("Use a browser's YouTube sign-in"),
            Some(
                "Sign in to YouTube in one of these browsers, then choose it here. Firefox works most \
                 reliably.\n\nMatinee uses that sign-in to play videos, and YouTube can then link what \
                 you watch to that account. A spare Google account works just as well.",
            ),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        dialog.set_extra_child(Some(&list));
        let this = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&this.window)).await.as_str() == "save" {
                let key = chosen.borrow().clone();
                this.use_sign_in(key);
            }
        });
    }

    /// Borrows the YouTube sign-in of the browser Preferences calls `key`, or
    /// stops. Whatever failed for want of it tries again.
    fn use_sign_in(self: &Rc<Self>, key: Option<String>) {
        tools::set_sign_in(key.as_deref());
        self.store.borrow_mut().cookies_browser = key;
        self.save_later();
        self.watch.sign_in_changed();
        let view = self.view.borrow().clone();
        if let View::Channel(id) = view {
            if !self.history_done.borrow().contains(&id) && !self.history_running(&id) {
                self.start_history(&id, false);
            }
        }
    }

    fn about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Matinee")
            .developer_name("Mxrphyy")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Follow YouTube channels without ads, tracking or an account.")
            .copyright("© 2026 Mxrphyy")
            .license_type(gtk::License::Gpl30)
            .website("https://github.com/Mxrphyyr/matinee")
            .issue_url("https://github.com/Mxrphyyr/matinee/issues")
            .build();
        if gtk::IconTheme::for_display(&WidgetExt::display(&self.window)).has_icon(APP_ID) {
            about.set_application_icon(APP_ID);
        }
        about.present(Some(&self.window));
    }
}

fn watch_hooks(weak: &Weak<Win>) -> WatchHooks {
    let (started, stopped, pop_out, volume, quality, sign_in) =
        (weak.clone(), weak.clone(), weak.clone(), weak.clone(), weak.clone(), weak.clone());
    WatchHooks {
        started: Box::new(move |video: &Video| {
            if let Some(win) = started.upgrade() {
                win.set_watched(&video.id, true);
            }
        }),
        stopped: Box::new(move |video: &Video, position: f64, duration: f64| {
            if let Some(win) = stopped.upgrade() {
                win.record_progress(video, position, duration);
            }
        }),
        pop_out: Box::new(move |video: &Video, at: f64| {
            if let Some(win) = pop_out.upgrade() {
                win.nav.pop();
                win.play_external(video, Some(at));
            }
        }),
        volume_changed: Box::new(move |level: f64| {
            if let Some(win) = volume.upgrade() {
                win.store.borrow_mut().volume = Some(level);
            }
        }),
        quality_changed: Box::new(move |chosen: Option<u32>| {
            if let Some(win) = quality.upgrade() {
                win.store.borrow_mut().quality = chosen;
                win.save_later();
            }
        }),
        choose_sign_in: Box::new(move || {
            if let Some(win) = sign_in.upgrade() {
                win.sign_in_dialog();
            }
        }),
    }
}

fn label(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.add_css_class(class);
    label
}

fn switch_row(title: &str, subtitle: &str, active: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).active(active).build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row
}

fn combo_row(title: &str, options: &[&str], selected: usize) -> adw::ComboRow {
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(options))
        .selected(selected as u32)
        .build()
}

fn skeleton_grid() -> gtk::FlowBox {
    let grid = gtk::FlowBox::builder()
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(8)
        .column_spacing(SPACING as u32)
        .row_spacing(34)
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .can_target(false)
        .can_focus(false)
        .build();
    grid.add_css_class("skeleton-grid");
    grid
}

/// A scrolled window whose content gets its natural height, so 16:9
/// placeholders keep their shape instead of collapsing to their minimum.
fn natural_scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let viewport = gtk::Viewport::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    viewport.set_vscroll_policy(gtk::ScrollablePolicy::Natural);
    viewport.set_child(Some(child));
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&viewport)
        .build()
}

enum RowIcon<'a> {
    Named(&'a str),
    Avatar(&'a str),
}

fn sidebar_row(icon: RowIcon, text: &str) -> (gtk::ListBoxRow, gtk::Label) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    match icon {
        RowIcon::Avatar(name) => content.append(&adw::Avatar::new(26, Some(name), true)),
        RowIcon::Named(name) => {
            let image = gtk::Image::from_icon_name(&icons::get(name));
            image.add_css_class("all-icon");
            content.append(&image);
        }
    }
    let name = gtk::Label::new(Some(text));
    name.add_css_class("channel-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);
    content.append(&name);
    let badge = gtk::Label::new(None);
    badge.add_css_class("count-badge");
    badge.set_valign(gtk::Align::Center);
    badge.set_visible(false);
    content.append(&badge);
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&content));
    (row, badge)
}

/// The "Channels" heading; activating it folds the list away.
fn section_row(count: usize, collapsed: bool) -> gtk::ListBoxRow {
    let title = gtk::Label::new(Some("Channels"));
    title.add_css_class("section-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let number = gtk::Label::new(Some(&count.to_string()));
    number.add_css_class("section-count");
    let chevron = gtk::Image::from_icon_name("pan-down-symbolic");
    chevron.add_css_class("chevron");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&title);
    content.append(&number);
    content.append(&chevron);
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&content));
    row.set_selectable(false);
    row.add_css_class("section-row");
    row.set_tooltip_text(Some("Show or hide your channels"));
    if collapsed {
        row.add_css_class("collapsed");
    }
    row
}

fn skeleton_card(index: usize, min_width: i32) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("skeleton");
    card.add_css_class(&format!("d{}", index % 4));
    card.set_size_request(min_width, -1);
    let thumb = gtk::Picture::for_paintable(&gdk::Paintable::new_empty(16, 9));
    thumb.set_can_shrink(true);
    thumb.set_content_fit(gtk::ContentFit::Fill);
    thumb.add_css_class("sk-thumb");
    card.append(&thumb);
    for (height, end_margin) in [(15, 24), (12, 110)] {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.add_css_class("sk-line");
        line.set_size_request(-1, height);
        line.set_margin_end(end_margin);
        card.append(&line);
    }
    card
}

fn logo_texture() -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from_static(LOGO_SVG)).ok()
}
