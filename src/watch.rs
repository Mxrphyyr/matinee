//! The embedded player. mpv draws into a GtkGLArea and Matinee's own controls
//! float over the picture, fading away while you watch. The picture can also
//! move to a small picture-in-picture window and keep playing while you browse.
//!
//! Opening a video is the app's second orchestrated moment: the page slides
//! in over the browser, the thumbnail waits dimmed and soft while the stream
//! loads, then the picture fades up as the poster fades out. The slide does no
//! heavy work itself: OpenGL and the renderer start once it has landed.

use crate::icons;
use crate::model::Video;
use crate::mpris::Status;
use crate::mpv::{self, Event, Format, Mpv, Render, Value};
use crate::{perf, tools, util};
use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const HIDE_CONTROLS_AFTER: Duration = Duration::from_millis(2600);
const HIDE_PIP_CONTROLS_AFTER: Duration = Duration::from_millis(1600);
const SPEEDS: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
/// The seek bar and clock refresh at most this often, and only while shown.
const PROGRESS_INTERVAL_US: i64 = 250_000;
/// How long the "up next" card waits before playing on.
const AUTOPLAY_SECONDS: u32 = 8;

pub struct WatchHooks {
    /// The first frame is on screen.
    pub started: Box<dyn Fn(&Video)>,
    /// The player closed at `position` of `duration` seconds.
    pub stopped: Box<dyn Fn(&Video, f64, f64)>,
    /// The viewer wants to continue in a separate mpv window, from this second.
    pub pop_out: Box<dyn Fn(&Video, f64)>,
    pub volume_changed: Box<dyn Fn(f64)>,
    /// A quality was picked in the player; remember it for next time.
    pub quality_changed: Box<dyn Fn(Option<u32>)>,
    /// The viewer wants to choose a browser whose YouTube sign-in to use.
    pub choose_sign_in: Box<dyn Fn()>,
    /// Playback changed: what's playing (None once stopped) and how.
    pub playback: Box<dyn Fn(Option<&Video>, Playback)>,
    /// The position jumped to this second.
    pub seeked: Box<dyn Fn(f64)>,
    /// The video after this one in the list it was opened from.
    pub next_video: Box<dyn Fn() -> Option<Video>>,
    pub play_next: Box<dyn Fn()>,
    pub play_previous: Box<dyn Fn()>,
    /// Fetches a video's thumbnail, for the "up next" card.
    pub thumbnail: Box<dyn Fn(&str, Box<dyn FnOnce(&gdk::Texture)>)>,
    /// Subtitles were turned on or off in the player; remember it.
    pub subtitles_changed: Box<dyn Fn(bool)>,
    /// Save this video for later, or remove it; says whether it's saved now.
    pub toggle_saved: Box<dyn Fn(&Video) -> bool>,
    pub is_saved: Box<dyn Fn(&str) -> bool>,
    /// The viewer wants yt-dlp updated (a video wouldn't play).
    pub update_yt_dlp: Box<dyn Fn()>,
}

/// How playback is going, for the desktop's media controls.
#[derive(Clone, Copy, PartialEq)]
pub struct Playback {
    pub status: Status,
    /// Seconds; 0 while unknown.
    pub duration: f64,
    pub speed: f64,
    /// 0 to 100.
    pub volume: f64,
}

/// A place mpv can draw: the player page or the picture-in-picture window.
/// libmpv allows one renderer at a time, so the picture moves between them.
struct Surface {
    area: gtk::GLArea,
    render: RefCell<Option<Render>>,
    size: Cell<(i32, i32)>,
}

impl Surface {
    fn new(class: &str) -> Rc<Self> {
        let area = gtk::GLArea::new();
        area.set_auto_render(false);
        area.set_has_depth_buffer(false);
        area.set_has_stencil_buffer(false);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.add_css_class(class);
        let surface = Rc::new(Self { area, render: RefCell::default(), size: Cell::new((1, 1)) });
        let weak = Rc::downgrade(&surface);
        surface.area.connect_resize(move |_, width, height| {
            if let Some(surface) = weak.upgrade() {
                surface.size.set((width.max(1), height.max(1)));
            }
        });
        let weak = Rc::downgrade(&surface);
        surface.area.connect_render(move |_, _| {
            if let Some(surface) = weak.upgrade() {
                if let Some(render) = surface.render.borrow().as_ref() {
                    // A slow draw is a frame with new shaders to compile.
                    let _timing = perf::span("video frame drawn", 8);
                    let (width, height) = surface.size.get();
                    render.draw(mpv::current_framebuffer(), width, height);
                }
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&surface);
        surface.area.connect_unrealize(move |_| {
            if let Some(surface) = weak.upgrade() {
                surface.release();
            }
        });
        surface
    }

    /// Gives up mpv's renderer, with this surface's OpenGL context current as
    /// libmpv requires.
    fn release(&self) {
        if self.render.borrow().is_none() {
            return;
        }
        if self.area.is_realized() {
            self.area.make_current();
        }
        self.render.borrow_mut().take();
    }

    fn is_attached(&self) -> bool {
        self.render.borrow().is_some()
    }
}

struct Pip {
    window: adw::Window,
    surface: Rc<Surface>,
    play: gtk::Button,
    buttons: Vec<gtk::Widget>,
    hide_source: RefCell<Option<glib::SourceId>>,
}

pub struct Watch {
    pub page: adw::NavigationPage,
    stage: gtk::Overlay,
    main: Rc<Surface>,
    poster: gtk::Picture,
    spinner: gtk::Spinner,
    problem: gtk::Box,
    problem_title: gtk::Label,
    problem_detail: gtk::Label,
    /// "Open in mpv", offered when the problem is this window's own.
    elsewhere: gtk::Button,
    /// Offered when a browser sign-in is the way past YouTube's refusal.
    choose_browser: gtk::Button,
    /// Offered when a video won't play for no reason Matinee recognises.
    update_button: gtk::Button,
    top: gtk::Revealer,
    bottom: gtk::Revealer,
    title: gtk::Label,
    channel: gtk::Label,
    /// "Save for later", in the top bar.
    save: gtk::Button,
    seek: gtk::Scale,
    time: gtk::Label,
    /// The current chapter's name, beside the clock.
    chapter: gtk::Label,
    play: gtk::Button,
    mute: gtk::Button,
    volume: gtk::Scale,
    speed: gtk::Button,
    subtitles_button: gtk::Button,
    quality_button: gtk::MenuButton,
    pip_button: gtk::Button,
    fullscreen: gtk::Button,
    start_over: gtk::Button,
    /// The "up next" card shown when a video ends with autoplay on.
    up_next: gtk::Box,
    up_next_thumb: gtk::Picture,
    up_next_title: gtk::Label,
    up_next_channel: gtk::Label,
    up_next_count: gtk::Label,

    hooks: WatchHooks,
    initial_volume: f64,
    /// Tallest video to stream; None matches the screen.
    quality: Cell<Option<u32>>,
    quality_action: gio::SimpleAction,
    mpv: RefCell<Option<Rc<Mpv>>>,
    pip: RefCell<Option<Rc<Pip>>>,
    video: RefCell<Option<Video>>,
    /// A URL waiting for a renderer before it can be loaded.
    pending: RefCell<Option<String>>,
    window: RefCell<glib::WeakRef<gtk::Window>>,
    /// The page has finished sliding in and is on screen.
    shown: Cell<bool>,
    /// The picture moved to another surface and video must be turned back on.
    reattach: Cell<bool>,
    duration: Cell<f64>,
    position: Cell<f64>,
    cache_time: Cell<f64>,
    paused: Cell<bool>,
    ended: Cell<bool>,
    started: Cell<bool>,
    muted: Cell<bool>,
    speed_index: Cell<usize>,
    over_controls: Cell<bool>,
    last_pointer: Cell<(f64, f64)>,
    seek_hold_until: Cell<i64>,
    seek_target: Cell<f64>,
    last_progress: Cell<i64>,
    time_text: RefCell<String>,
    hide_source: RefCell<Option<glib::SourceId>>,
    click_source: RefCell<Option<glib::SourceId>>,
    last_error: RefCell<String>,
    /// Bumped on every open, so late events from an earlier video are ignored.
    load_generation: Cell<u64>,
    /// Where this video resumed from, or 0.
    resume_at: Cell<f64>,
    /// Keeps the screen awake while a video plays.
    inhibit_cookie: Cell<u32>,
    /// A video is open (loading, playing, paused or ended), as opposed to stopped.
    open: Cell<bool>,
    /// The video's chapters: start time and title.
    chapters: RefCell<Vec<(f64, String)>>,
    subtitles: Cell<bool>,
    autoplay: Cell<bool>,
    /// The video the "up next" card offers, while its countdown runs.
    up_next_video: RefCell<Option<Video>>,
    countdown_source: RefCell<Option<glib::SourceId>>,
    countdown_left: Cell<u32>,
}

impl Watch {
    pub fn new(initial_volume: f64, quality: Option<u32>, autoplay: bool, subtitles: bool, hooks: WatchHooks) -> Rc<Self> {
        let main = Surface::new("video");
        // Hidden until the page has slid in, so the slide never waits on OpenGL.
        main.area.set_visible(false);

        let poster = gtk::Picture::new();
        poster.set_content_fit(gtk::ContentFit::Cover);
        poster.set_can_shrink(true);
        poster.set_can_target(false);
        poster.add_css_class("poster");

        let spinner = gtk::Spinner::new();
        spinner.set_size_request(44, 44);
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        spinner.set_can_target(false);

        // ── Problem card ──
        let problem_icon = gtk::Image::from_icon_name(&icons::get("dialog-warning-symbolic"));
        problem_icon.set_pixel_size(36);
        let problem_title = gtk::Label::new(None);
        problem_title.add_css_class("problem-title");
        let problem_detail = gtk::Label::new(None);
        problem_detail.set_wrap(true);
        problem_detail.set_justify(gtk::Justification::Center);
        problem_detail.set_max_width_chars(54);
        problem_detail.add_css_class("problem-detail");
        let retry = gtk::Button::with_label("Try again");
        retry.add_css_class("pill");
        retry.add_css_class("suggested-action");
        let elsewhere = gtk::Button::with_label("Open in mpv");
        elsewhere.add_css_class("pill");
        let update_button = gtk::Button::with_label("Update yt-dlp");
        update_button.add_css_class("pill");
        update_button.set_visible(false);
        let choose_browser = gtk::Button::with_label("Choose a browser…");
        choose_browser.add_css_class("pill");
        choose_browser.set_visible(false);
        let problem_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        problem_buttons.set_halign(gtk::Align::Center);
        problem_buttons.set_margin_top(8);
        problem_buttons.append(&retry);
        problem_buttons.append(&elsewhere);
        problem_buttons.append(&update_button);
        problem_buttons.append(&choose_browser);
        let problem = gtk::Box::new(gtk::Orientation::Vertical, 10);
        problem.add_css_class("problem");
        problem.set_halign(gtk::Align::Center);
        problem.set_valign(gtk::Align::Center);
        problem.set_visible(false);
        problem.append(&problem_icon);
        problem.append(&problem_title);
        problem.append(&problem_detail);
        problem.append(&problem_buttons);

        // ── Top bar ──
        let back = icon_button("go-previous-symbolic", "Back (Esc)");
        back.set_action_name(Some("navigation.pop"));
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.add_css_class("watch-title");
        let channel = gtk::Label::new(None);
        channel.set_xalign(0.0);
        channel.set_ellipsize(pango::EllipsizeMode::End);
        channel.add_css_class("watch-channel");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 1);
        titles.set_hexpand(true);
        titles.set_valign(gtk::Align::Center);
        titles.append(&title);
        titles.append(&channel);
        let pop_out = gtk::Button::with_label("Open in mpv");
        pop_out.add_css_class("flat");
        pop_out.set_valign(gtk::Align::Center);
        pop_out.set_tooltip_text(Some("Continue in a separate mpv window"));
        let save = icon_button("bookmark-new-symbolic", "Save for later (S)");
        save.add_css_class("save");
        let top_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        top_bar.add_css_class("watch-bar");
        top_bar.append(&back);
        top_bar.append(&titles);
        top_bar.append(&save);
        top_bar.append(&pop_out);
        let top = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(260)
            .valign(gtk::Align::Start)
            .reveal_child(true)
            .child(&top_bar)
            .build();

        // ── Controls ──
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.set_show_fill_level(true);
        seek.set_restrict_to_fill_level(false);
        seek.set_fill_level(0.0);
        seek.add_css_class("watch-seek");
        let play = icon_button("media-playback-pause-symbolic", "Pause (Space)");
        let time = gtk::Label::new(Some("0:00"));
        time.add_css_class("watch-time");
        let chapter = gtk::Label::new(None);
        chapter.add_css_class("watch-chapter");
        chapter.set_ellipsize(pango::EllipsizeMode::End);
        chapter.set_max_width_chars(40);
        chapter.set_valign(gtk::Align::Center);
        chapter.set_visible(false);
        let start_over = gtk::Button::with_label("Start over");
        start_over.add_css_class("start-over");
        start_over.set_valign(gtk::Align::Center);
        start_over.set_visible(false);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let mute = icon_button("audio-volume-high-symbolic", "Mute (M)");
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        volume.set_draw_value(false);
        volume.set_value(initial_volume.clamp(0.0, 100.0));
        volume.set_size_request(96, -1);
        volume.set_valign(gtk::Align::Center);
        volume.add_css_class("watch-volume");
        let speed = gtk::Button::with_label("1×");
        speed.add_css_class("flat");
        speed.add_css_class("speed");
        speed.set_valign(gtk::Align::Center);
        speed.set_tooltip_text(Some("Playback speed"));
        let subtitles_button = icon_button("media-view-subtitles-symbolic", "Subtitles (C)");
        subtitles_button.add_css_class("subtitles");
        if subtitles {
            subtitles_button.add_css_class("active");
        }
        let quality_menu = gio::Menu::new();
        for (key, label) in QUALITY_CHOICES {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some("watch.quality"), Some(&key.to_variant()));
            quality_menu.append_item(&item);
        }
        let quality_button = gtk::MenuButton::builder()
            .label("Auto")
            .menu_model(&quality_menu)
            .direction(gtk::ArrowType::Up)
            .tooltip_text("Video quality")
            .valign(gtk::Align::Center)
            .build();
        quality_button.add_css_class("flat");
        quality_button.add_css_class("quality");
        let pip_button = icon_button("picture-in-picture-symbolic", "Picture in picture (P)");
        let fullscreen = icon_button("view-fullscreen-symbolic", "Full screen (F)");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for widget in [
            play.upcast_ref::<gtk::Widget>(),
            time.upcast_ref(),
            chapter.upcast_ref(),
            start_over.upcast_ref(),
            spacer.upcast_ref(),
            mute.upcast_ref(),
            volume.upcast_ref(),
            speed.upcast_ref(),
            subtitles_button.upcast_ref(),
            quality_button.upcast_ref(),
            pip_button.upcast_ref(),
            fullscreen.upcast_ref(),
        ] {
            row.append(widget);
        }
        let controls = gtk::Box::new(gtk::Orientation::Vertical, 2);
        controls.add_css_class("watch-controls");
        controls.append(&seek);
        controls.append(&row);
        let bottom = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .transition_duration(260)
            .valign(gtk::Align::End)
            .reveal_child(true)
            .child(&controls)
            .build();

        // ── Up next ──
        let up_next_thumb = gtk::Picture::new();
        up_next_thumb.set_size_request(160, 90);
        up_next_thumb.set_content_fit(gtk::ContentFit::Cover);
        up_next_thumb.set_can_shrink(true);
        up_next_thumb.add_css_class("up-next-thumb");
        let up_next_heading = gtk::Label::new(Some("Up next"));
        up_next_heading.add_css_class("up-next-heading");
        up_next_heading.set_xalign(0.0);
        let up_next_title = gtk::Label::new(None);
        up_next_title.add_css_class("up-next-title");
        up_next_title.set_xalign(0.0);
        up_next_title.set_wrap(true);
        up_next_title.set_lines(2);
        up_next_title.set_ellipsize(pango::EllipsizeMode::End);
        up_next_title.set_max_width_chars(30);
        let up_next_channel = gtk::Label::new(None);
        up_next_channel.add_css_class("up-next-channel");
        up_next_channel.set_xalign(0.0);
        up_next_channel.set_ellipsize(pango::EllipsizeMode::End);
        let up_next_count = gtk::Label::new(None);
        up_next_count.add_css_class("up-next-count");
        up_next_count.set_xalign(0.0);
        let play_now = gtk::Button::with_label("Play now");
        play_now.add_css_class("pill");
        play_now.add_css_class("suggested-action");
        let cancel_next = gtk::Button::with_label("Cancel");
        cancel_next.add_css_class("pill");
        let up_next_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        up_next_buttons.set_margin_top(6);
        up_next_buttons.append(&play_now);
        up_next_buttons.append(&cancel_next);
        let up_next_text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        up_next_text.set_valign(gtk::Align::Center);
        up_next_text.append(&up_next_heading);
        up_next_text.append(&up_next_title);
        up_next_text.append(&up_next_channel);
        up_next_text.append(&up_next_count);
        up_next_text.append(&up_next_buttons);
        let up_next = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        up_next.add_css_class("up-next");
        up_next.set_halign(gtk::Align::End);
        up_next.set_valign(gtk::Align::End);
        up_next.set_margin_end(24);
        up_next.set_margin_bottom(100);
        up_next.set_visible(false);
        up_next.append(&up_next_thumb);
        up_next.append(&up_next_text);

        let stage = gtk::Overlay::new();
        stage.add_css_class("watch");
        stage.set_focusable(true);
        stage.set_child(Some(&main.area));
        stage.add_overlay(&poster);
        stage.add_overlay(&spinner);
        stage.add_overlay(&problem);
        stage.add_overlay(&up_next);
        stage.add_overlay(&top);
        stage.add_overlay(&bottom);

        let page = adw::NavigationPage::builder()
            .child(&stage)
            .title("Watching")
            .tag("watch")
            .build();
        let quality_action =
            gio::SimpleAction::new_stateful("quality", Some(glib::VariantTy::STRING), &quality_key(quality).to_variant());
        let actions = gio::SimpleActionGroup::new();
        actions.add_action(&quality_action);
        page.insert_action_group("watch", Some(&actions));

        let watch = Rc::new(Self {
            page,
            stage,
            main,
            poster,
            spinner,
            problem,
            problem_title,
            problem_detail,
            elsewhere,
            choose_browser,
            update_button,
            top,
            bottom,
            title,
            channel,
            save,
            seek,
            time,
            chapter,
            play,
            mute,
            volume,
            speed,
            subtitles_button,
            quality_button,
            pip_button,
            fullscreen,
            start_over,
            up_next,
            up_next_thumb,
            up_next_title,
            up_next_channel,
            up_next_count,
            hooks,
            initial_volume,
            quality: Cell::new(quality),
            quality_action,
            mpv: RefCell::default(),
            pip: RefCell::default(),
            video: RefCell::default(),
            pending: RefCell::default(),
            window: RefCell::default(),
            shown: Cell::new(false),
            reattach: Cell::new(false),
            duration: Cell::new(0.0),
            position: Cell::new(0.0),
            cache_time: Cell::new(0.0),
            paused: Cell::new(false),
            ended: Cell::new(false),
            started: Cell::new(false),
            muted: Cell::new(false),
            speed_index: Cell::new(0),
            over_controls: Cell::new(false),
            last_pointer: Cell::new((-1.0, -1.0)),
            seek_hold_until: Cell::new(0),
            seek_target: Cell::new(0.0),
            last_progress: Cell::new(0),
            time_text: RefCell::default(),
            hide_source: RefCell::default(),
            click_source: RefCell::default(),
            last_error: RefCell::default(),
            load_generation: Cell::new(0),
            resume_at: Cell::new(0.0),
            inhibit_cookie: Cell::new(0),
            open: Cell::new(false),
            chapters: RefCell::default(),
            subtitles: Cell::new(subtitles),
            autoplay: Cell::new(autoplay),
            up_next_video: RefCell::default(),
            countdown_source: RefCell::default(),
            countdown_left: Cell::new(0),
        });
        watch.connect(&retry, &pop_out, &play_now, &cancel_next, &top_bar, &controls);
        watch
    }

    fn connect(
        self: &Rc<Self>,
        retry: &gtk::Button,
        pop_out: &gtk::Button,
        play_now: &gtk::Button,
        cancel_next: &gtk::Button,
        top_bar: &gtk::Box,
        controls: &gtk::Box,
    ) {
        // ── Buttons ──
        let on = |button: &gtk::Button, action: fn(&Rc<Watch>)| {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(watch) = weak.upgrade() {
                    action(&watch);
                }
            });
        };
        on(&self.play, |w| w.toggle_pause());
        on(&self.mute, |w| w.toggle_mute());
        on(&self.speed, |w| w.next_speed());
        on(&self.pip_button, |w| w.enter_pip());
        let weak = Rc::downgrade(self);
        self.quality_action.connect_activate(move |action, param| {
            let Some(key) = param.and_then(|p| p.get::<String>()) else { return };
            action.set_state(&key.to_variant());
            if let Some(watch) = weak.upgrade() {
                watch.choose_quality(key.parse().ok());
            }
        });
        on(&self.fullscreen, |w| w.toggle_fullscreen());
        on(&self.start_over, |w| w.start_over());
        on(retry, |w| w.retry());
        on(&self.elsewhere, |w| w.pop_out());
        on(&self.choose_browser, |w| (w.hooks.choose_sign_in)());
        on(&self.update_button, |w| {
            w.update_button.set_sensitive(false);
            w.update_button.set_label("Updating…");
            (w.hooks.update_yt_dlp)();
        });
        on(pop_out, |w| w.pop_out());
        on(&self.subtitles_button, |w| w.toggle_subtitles());
        on(&self.save, |w| w.toggle_saved());
        on(play_now, |w| w.play_next_now());
        on(cancel_next, |w| w.cancel_autoplay());

        let weak = Rc::downgrade(self);
        self.seek.connect_change_value(move |_, _, value| {
            if let Some(watch) = weak.upgrade() {
                watch.seek_to(value);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.volume.connect_change_value(move |_, _, value| {
            if let Some(watch) = weak.upgrade() {
                watch.set_volume(value);
            }
            glib::Propagation::Proceed
        });

        // ── Pointer: controls appear on movement and leave when idle ──
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            let Some(watch) = weak.upgrade() else { return };
            // Some compositors repeat the last position; only real movement counts.
            if watch.last_pointer.replace((x, y)) != (x, y) {
                watch.show_controls();
            }
        });
        self.stage.add_controller(motion);
        for bar in [top_bar.upcast_ref::<gtk::Widget>(), controls.upcast_ref()] {
            let hover = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(self);
            hover.connect_enter(move |_, _, _| {
                if let Some(watch) = weak.upgrade() {
                    watch.over_controls.set(true);
                }
            });
            let weak = Rc::downgrade(self);
            hover.connect_leave(move |_| {
                if let Some(watch) = weak.upgrade() {
                    watch.over_controls.set(false);
                }
            });
            bar.add_controller(hover);
        }

        // ── Click to pause, double-click for full screen ──
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, n_press, _, _| {
            let Some(watch) = weak.upgrade() else { return };
            if let Some(source) = watch.click_source.borrow_mut().take() {
                source.remove();
            }
            if n_press >= 2 {
                watch.toggle_fullscreen();
                return;
            }
            // Wait briefly so a double-click doesn't also pause.
            let weak = Rc::downgrade(&watch);
            let source = glib::timeout_add_local_once(Duration::from_millis(240), move || {
                if let Some(watch) = weak.upgrade() {
                    watch.click_source.borrow_mut().take();
                    watch.toggle_pause();
                }
            });
            *watch.click_source.borrow_mut() = Some(source);
        });
        self.main.area.add_controller(click);

        // ── Keyboard ──
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(watch) = weak.upgrade() else { return glib::Propagation::Proceed };
            let modifiers =
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK;
            if state.intersects(modifiers) {
                return glib::Propagation::Proceed;
            }
            let handled = match key {
                gdk::Key::space | gdk::Key::k | gdk::Key::K => {
                    watch.toggle_pause();
                    true
                }
                gdk::Key::Left => {
                    watch.seek_by(-5.0);
                    true
                }
                gdk::Key::Right => {
                    watch.seek_by(5.0);
                    true
                }
                gdk::Key::j | gdk::Key::J => {
                    watch.seek_by(-10.0);
                    true
                }
                gdk::Key::l | gdk::Key::L => {
                    watch.seek_by(10.0);
                    true
                }
                gdk::Key::Up => {
                    watch.set_volume(watch.volume.value() + 5.0);
                    true
                }
                gdk::Key::Down => {
                    watch.set_volume(watch.volume.value() - 5.0);
                    true
                }
                gdk::Key::f | gdk::Key::F => {
                    watch.toggle_fullscreen();
                    true
                }
                gdk::Key::p | gdk::Key::P => {
                    watch.enter_pip();
                    true
                }
                gdk::Key::m | gdk::Key::M => {
                    watch.toggle_mute();
                    true
                }
                gdk::Key::c | gdk::Key::C => {
                    watch.toggle_subtitles();
                    true
                }
                gdk::Key::s | gdk::Key::S => {
                    watch.toggle_saved();
                    true
                }
                gdk::Key::n => {
                    watch.play_next_now();
                    true
                }
                gdk::Key::N => {
                    (watch.hooks.play_previous)();
                    true
                }
                gdk::Key::Page_Down => {
                    watch.step_chapter(1);
                    true
                }
                gdk::Key::Page_Up => {
                    watch.step_chapter(-1);
                    true
                }
                gdk::Key::Home => {
                    watch.seek_to(0.0);
                    true
                }
                gdk::Key::Escape => {
                    if watch.is_fullscreen() {
                        watch.set_fullscreen(false);
                    } else {
                        let _ = watch.page.activate_action("navigation.pop", None);
                    }
                    true
                }
                _ => false,
            };
            if handled {
                watch.show_controls();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.page.add_controller(keys);

        // ── The page's arrival and departure ──
        let weak = Rc::downgrade(self);
        self.page.connect_shown(move |_| {
            if let Some(watch) = weak.upgrade() {
                watch.shown.set(true);
                watch.begin();
            }
        });
        let weak = Rc::downgrade(self);
        self.page.connect_hidden(move |_| {
            if let Some(watch) = weak.upgrade() {
                watch.shown.set(false);
            }
        });

        // Remember the window, and keep the full-screen button in step with it.
        let weak = Rc::downgrade(self);
        self.page.connect_map(move |page| {
            let Some(watch) = weak.upgrade() else { return };
            if watch.window.borrow().upgrade().is_some() {
                return;
            }
            if let Some(window) = page.root().and_downcast::<gtk::Window>() {
                watch.window.borrow().set(Some(&window));
                let weak = Rc::downgrade(&watch);
                window.connect_fullscreened_notify(move |window| {
                    if let Some(watch) = weak.upgrade() {
                        watch.sync_fullscreen(window.is_fullscreen());
                    }
                });
            }
        });
    }

    // ── Playback ───────────────────────────────────────────────────────

    /// Plays `video`, from `resume` seconds in when given. If the page is still
    /// sliding in, playback starts once it lands.
    pub fn open(self: &Rc<Self>, video: &Video, resume: Option<f64>) {
        let same = self.video.borrow().as_ref().is_some_and(|v| v.id == video.id);
        if !same {
            self.poster.set_paintable(None::<&gdk::Paintable>);
        }
        self.load_generation.set(self.load_generation.get() + 1);
        self.resume_at.set(resume.unwrap_or(0.0));
        self.start_over.set_visible(false);
        self.cancel_autoplay();
        self.chapters.borrow_mut().clear();
        self.seek.clear_marks();
        self.chapter.set_visible(false);
        self.open.set(true);
        *self.video.borrow_mut() = Some(video.clone());
        self.page.set_title(&video.title);
        self.title.set_label(&video.title);
        self.channel.set_label(&video.channel_name);
        self.show_saved((self.hooks.is_saved)(&video.id));
        self.poster.set_visible(true);
        self.poster.remove_css_class("gone");
        self.main.area.remove_css_class("playing");
        self.problem.set_visible(false);
        self.started.set(false);
        self.ended.set(false);
        self.paused.set(false);
        self.duration.set(0.0);
        self.position.set(0.0);
        self.cache_time.set(0.0);
        self.seek.set_range(0.0, 1.0);
        self.seek.set_value(0.0);
        self.seek.set_fill_level(0.0);
        self.last_error.borrow_mut().clear();
        self.refresh_progress(true);
        self.update_play_button();
        self.set_busy(true);
        self.show_controls();
        *self.pending.borrow_mut() = Some(video.watch_url());
        if self.in_pip() || self.shown.get() {
            self.begin();
        }
        self.stage.grab_focus();
        self.notify_playback();
    }

    /// Shows a soft, dimmed version of the thumbnail while the stream loads.
    /// Shrinking it to a few dozen pixels and letting the GPU stretch it back
    /// looks like a heavy blur but costs nothing per frame.
    pub fn set_poster(self: &Rc<Self>, video_id: &str, texture: &gdk::Texture) {
        let texture = texture.clone();
        let video_id = video_id.to_string();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(Some(soft)) = gio::spawn_blocking(move || soften(&texture)).await else { return };
            let Some(watch) = weak.upgrade() else { return };
            if watch.video.borrow().as_ref().is_some_and(|v| v.id == video_id) {
                watch.poster.set_paintable(Some(&soft));
            }
        });
    }

    pub fn in_pip(&self) -> bool {
        self.pip.borrow().is_some()
    }

    /// Starts mpv and its renderer ahead of time, so the first video opens
    /// without a pause: creating the renderer took 0.2 s on the owner's
    /// machine. The page isn't on screen, so nothing is drawn yet.
    pub fn warm_up(self: &Rc<Self>) {
        if self.ensure_mpv().is_none() || self.in_pip() || self.shown.get() {
            return;
        }
        self.main.area.set_visible(true);
        self.attach(&self.main);
    }

    /// The player page left the screen. Playback stops unless the picture
    /// moved to its own window.
    pub fn page_closed(&self) {
        if !self.in_pip() {
            self.stop();
        }
    }

    pub fn stop(&self) {
        if self.started.replace(false) {
            let video = self.video.borrow().clone();
            if let Some(video) = video {
                (self.hooks.stopped)(&video, self.position.get(), self.duration.get());
            }
        }
        self.open.set(false);
        self.cancel_autoplay();
        self.pending.borrow_mut().take();
        if let Some(mpv) = self.mpv() {
            let _ = mpv.command(&["stop"]);
        }
        for slot in [&self.hide_source, &self.click_source] {
            if let Some(source) = slot.borrow_mut().take() {
                source.remove();
            }
        }
        self.notify_playback();
        self.set_fullscreen(false);
        self.set_inhibit(false);
        self.stage.set_cursor(None);
        self.set_busy(false);
        let pip = self.pip.borrow_mut().take();
        if let Some(pip) = pip {
            pip.surface.release();
            pip.window.destroy();
        }
        // The page's renderer stays, ready for the next video.
        self.main.area.set_visible(false);
    }

    fn mpv(&self) -> Option<Rc<Mpv>> {
        self.mpv.borrow().clone()
    }

    fn ensure_mpv(self: &Rc<Self>) -> Option<Rc<Mpv>> {
        if let Some(mpv) = self.mpv() {
            return Some(mpv);
        }
        let (tx, rx) = async_channel::bounded::<()>(1);
        let volume = format!("{:.0}", self.initial_volume.clamp(0.0, 100.0));
        let script_opts = ytdl_script_opts();
        // Compiled shaders are kept on disk, so after the first video the
        // first frame needn't wait for the compiler.
        let shader_cache = glib::user_cache_dir().join("matinee").join("shaders");
        let _ = std::fs::create_dir_all(&shader_cache);
        let shader_cache = shader_cache.to_string_lossy().into_owned();
        let options: Vec<(&str, &str)> = vec![
            ("vo", "libmpv"),
            // Hardware decoding with a copy back to memory works everywhere.
            // NVIDIA's decoder comes first (mpv's own choice was a Vulkan
            // decoder that was slower and may run on another chip), then
            // VA-API for Intel and AMD, then whatever mpv finds safe. An
            // older libmpv takes only the last. MATINEE_HWDEC_INTEROP=1
            // tries zero-copy decoding instead.
            ("hwdec", if interop() { "nvdec,vaapi,auto-safe" } else { "nvdec-copy,vaapi-copy,auto-copy-safe" }),
            ("ytdl", "yes"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("video-timing-offset", "0"),
            ("audio-client-name", "Matinee"),
            // A missing or busy sound device shouldn't stop the video.
            ("audio-fallback-to-null", "yes"),
            ("volume", &volume),
            ("script-opts", &script_opts),
            ("gpu-shader-cache-dir", &shader_cache),
        ];
        match Mpv::new(&options, move || {
            let _ = tx.try_send(());
        }) {
            Ok(mpv) => {
                let mpv = Rc::new(mpv);
                for (name, format) in [
                    ("time-pos", Format::Double),
                    ("duration", Format::Double),
                    ("demuxer-cache-time", Format::Double),
                    ("pause", Format::Flag),
                    ("paused-for-cache", Format::Flag),
                    ("eof-reached", Format::Flag),
                    ("volume", Format::Double),
                    ("mute", Format::Flag),
                    ("speed", Format::Double),
                    ("height", Format::Double),
                    ("chapter-list", Format::None),
                    ("chapter", Format::None),
                ] {
                    mpv.observe(name, format);
                }
                mpv.request_log_messages("error");
                let weak = Rc::downgrade(self);
                glib::spawn_future_local(async move {
                    while rx.recv().await.is_ok() {
                        let Some(watch) = weak.upgrade() else { break };
                        watch.drain_events();
                    }
                });
                if perf::enabled() {
                    // How playback is going, every few seconds.
                    let weak = Rc::downgrade(self);
                    glib::timeout_add_seconds_local(5, move || {
                        let Some(watch) = weak.upgrade() else { return glib::ControlFlow::Break };
                        if let Some(mpv) = watch.mpv().filter(|_| watch.started.get()) {
                            let get = |name: &str| mpv.string(name).unwrap_or_else(|| "?".into());
                            perf::mark(&format!(
                                "player: {} frames dropped, {} shown late, {} s buffered, waiting for data: {}",
                                get("frame-drop-count"),
                                get("vo-delayed-frame-count"),
                                get("demuxer-cache-duration"),
                                get("paused-for-cache")
                            ));
                        }
                        glib::ControlFlow::Continue
                    });
                }
                *self.mpv.borrow_mut() = Some(mpv.clone());
                Some(mpv)
            }
            Err(e) => {
                self.show_problem("The video engine didn't start", &e);
                None
            }
        }
    }

    fn active_surface(&self) -> Rc<Surface> {
        match self.pip.borrow().as_ref() {
            Some(pip) => pip.surface.clone(),
            None => self.main.clone(),
        }
    }

    /// Puts the picture on whichever surface is active and loads anything
    /// waiting. Runs once the page has landed, or right away in picture in
    /// picture.
    fn begin(self: &Rc<Self>) {
        let surface = self.active_surface();
        if Rc::ptr_eq(&surface, &self.main) {
            self.main.area.set_visible(true);
        }
        if !self.attach(&surface) {
            return;
        }
        if self.reattach.replace(false) {
            // Video was switched off for the move; turn it back on here.
            if let Some(mpv) = self.mpv() {
                mpv.set_string("vid", "auto");
            }
        }
        self.load_pending();
    }

    /// Gives `surface` mpv's renderer, taking it from the other surface.
    fn attach(self: &Rc<Self>, surface: &Rc<Surface>) -> bool {
        if surface.is_attached() {
            return true;
        }
        if !surface.area.is_realized() {
            surface.area.realize();
            if !surface.area.is_realized() {
                return false;
            }
        }
        // libmpv allows one renderer at a time.
        self.main.release();
        let pip = self.pip.borrow().clone();
        if let Some(pip) = pip {
            pip.surface.release();
        }
        surface.area.make_current();
        if perf::enabled() {
            perf::mark(&format!("OpenGL drawn by {}", mpv::renderer_name().unwrap_or_else(|| "?".into())));
        }
        if let Some(error) = surface.area.error() {
            self.show_problem(
                "Video can't be shown here",
                &format!(
                    "OpenGL isn't available ({}). Turn on “Play in a separate window” in Preferences.",
                    error.message()
                ),
            );
            return false;
        }
        let Some(mpv) = self.ensure_mpv() else { return false };
        let (tx, rx) = async_channel::bounded::<()>(1);
        let display = if interop() {
            mpv::native_display(&surface.area.display())
        } else {
            mpv::NativeDisplay::Unknown
        };
        match Render::new(mpv, display, move || {
            let _ = tx.try_send(());
        }) {
            Ok(render) => {
                *surface.render.borrow_mut() = Some(render);
                perf::mark("mpv renderer attached");
                let area = surface.area.downgrade();
                glib::spawn_future_local(async move {
                    while rx.recv().await.is_ok() {
                        match area.upgrade() {
                            Some(area) => area.queue_render(),
                            None => break,
                        }
                    }
                });
                true
            }
            Err(e) => {
                self.show_problem(
                    "Video can't be shown here",
                    &format!("mpv couldn't draw into the window ({e}). Turn on “Play in a separate window” in Preferences."),
                );
                false
            }
        }
    }

    fn load_pending(self: &Rc<Self>) {
        if !self.active_surface().is_attached() {
            return;
        }
        let Some(url) = self.pending.borrow_mut().take() else { return };
        let Some(mpv) = self.mpv() else { return };
        mpv.set_flag("pause", false);
        mpv.set_double("speed", SPEEDS[self.speed_index.get()]);
        mpv.set_string("ytdl-format", &self.format());
        mpv.set_string("script-opts", &ytdl_script_opts());
        // The browser sign-in, if it's turned on in Preferences, and which
        // subtitles yt-dlp should list.
        mpv.set_string("ytdl-raw-options", &tools::mpv_ytdl_options());
        // Subtitles start hidden; the button (or the remembered choice) shows them.
        mpv.set_string("sid", "no");
        // Applies to this file only; reset once it has loaded.
        let start = self.resume_at.get();
        mpv.set_string("start", &if start > 0.0 { format!("{start:.1}") } else { "none".to_string() });
        // MATINEE_PLAY_FILE plays a local file in place of every video: for
        // trying the player where YouTube can't be reached.
        let url = std::env::var("MATINEE_PLAY_FILE").ok().filter(|f| !f.is_empty()).unwrap_or(url);
        if let Err(e) = mpv.command(&["loadfile", &url, "replace"]) {
            self.show_problem("This video won't play", &e);
        }
    }

    fn drain_events(self: &Rc<Self>) {
        let Some(mpv) = self.mpv() else { return };
        while let Some(event) = mpv.next_event() {
            match event {
                Event::PlaybackRestart => self.on_frame_ready(),
                Event::FileLoaded => {
                    mpv.set_string("start", "none");
                    // A saved spot past the end (the video was replaced) starts over.
                    let duration = mpv.double("duration").unwrap_or(0.0);
                    if duration > 0.0 && self.resume_at.get() >= duration - 1.0 {
                        self.resume_at.set(0.0);
                        mpv.command_async(&["seek", "0", "absolute"]);
                    }
                    if self.subtitles.get() {
                        self.select_subtitles(&mpv);
                    }
                    self.read_chapters(&mpv);
                }
                Event::EndFile { failed: true, code } => {
                    // mpv delivers log lines after other events; give yt-dlp's
                    // explanation a moment to arrive before showing the problem.
                    let generation = self.load_generation.get();
                    let weak = Rc::downgrade(self);
                    glib::timeout_add_local_once(Duration::from_millis(250), move || {
                        if let Some(watch) = weak.upgrade() {
                            if watch.load_generation.get() == generation {
                                watch.on_failed(code);
                            }
                        }
                    });
                }
                Event::Property { name, value } => self.on_property(&name, value),
                Event::Log { prefix, text } => {
                    let text = text.trim();
                    // yt-dlp's own "ERROR: …" line explains the most; mpv follows it
                    // with a generic summary that shouldn't replace it.
                    let from_yt_dlp = prefix == "ytdl_hook" && text.contains("ERROR:");
                    // Hardware-decoder probes log errors on the way to a working
                    // fallback; only loading problems explain a failure.
                    let about_loading = ["ytdl_hook", "cplayer", "stream", "demux", "lavf", "ffmpeg"]
                        .iter()
                        .any(|p| prefix.starts_with(p));
                    let hardware_probe = ["AVHWDeviceContext", "libcuda", "vaapi", "libva", "vdpau", "VDPAU"]
                        .iter()
                        .any(|word| text.contains(word));
                    if !text.is_empty()
                        && about_loading
                        && !hardware_probe
                        && (from_yt_dlp || self.last_error.borrow().is_empty())
                    {
                        *self.last_error.borrow_mut() = util::tidy_error(text);
                    }
                }
                Event::Shutdown => break,
                _ => {}
            }
        }
    }

    fn on_property(self: &Rc<Self>, name: &str, value: Value) {
        match (name, value) {
            ("time-pos", Value::Double(seconds)) => {
                self.position.set(seconds);
                // The offer to start over fades once the video is well under way.
                if self.start_over.is_visible() && seconds > self.resume_at.get() + 15.0 {
                    self.start_over.set_visible(false);
                }
                self.refresh_progress(false);
            }
            ("duration", Value::Double(seconds)) => {
                self.duration.set(seconds);
                self.seek.set_range(0.0, seconds.max(0.1));
                self.refresh_progress(true);
                self.apply_chapter_marks();
                self.notify_playback();
            }
            ("demuxer-cache-time", Value::Double(seconds)) => self.cache_time.set(seconds),
            ("pause", Value::Flag(paused)) => {
                self.paused.set(paused);
                self.update_play_button();
                self.set_inhibit(!paused && self.started.get());
                self.show_controls();
                self.notify_playback();
            }
            ("eof-reached", Value::Flag(ended)) => {
                self.ended.set(ended);
                self.update_play_button();
                if ended {
                    self.set_inhibit(false);
                }
                self.show_controls();
                self.notify_playback();
                if ended && self.started.get() {
                    self.on_ended();
                }
            }
            ("chapter-list", _) => {
                if let Some(mpv) = self.mpv() {
                    self.read_chapters(&mpv);
                }
            }
            ("chapter", _) => self.update_chapter_label(),
            ("paused-for-cache", Value::Flag(waiting)) => {
                if self.started.get() {
                    self.set_busy(waiting);
                }
            }
            ("volume", Value::Double(volume)) => {
                self.volume.set_value(volume);
                self.update_mute_button();
                (self.hooks.volume_changed)(volume);
                self.notify_playback();
            }
            ("mute", Value::Flag(muted)) => {
                self.muted.set(muted);
                self.update_mute_button();
            }
            ("speed", Value::Double(speed)) => {
                self.speed.set_label(&speed_label(speed));
                self.notify_playback();
            }
            ("height", Value::Double(height)) => self.quality_button.set_label(&height_label(height)),
            _ => {}
        }
    }

    /// A frame is ready: after loading, the lights go down and the picture comes up.
    fn on_frame_ready(self: &Rc<Self>) {
        self.set_busy(false);
        if self.started.replace(true) {
            return;
        }
        if let Some(mpv) = self.mpv().filter(|_| perf::enabled()) {
            // By the first frame the decoder is running, so this is exact.
            let get = |name: &str| mpv.string(name).unwrap_or_else(|| "?".into());
            perf::mark(&format!(
                "playing: {} {}x{} at {} fps, {} {}, decoded by {}",
                get("video-codec"),
                get("video-params/w"),
                get("video-params/h"),
                get("container-fps"),
                get("video-params/pixelformat"),
                get("video-params/primaries"),
                get("hwdec-current")
            ));
        }
        self.main.area.add_css_class("playing");
        self.poster.add_css_class("gone");
        // Once faded out, the poster leaves the scene entirely.
        let generation = self.load_generation.get();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(1100), move || {
            if let Some(watch) = weak.upgrade() {
                if watch.load_generation.get() == generation && watch.started.get() {
                    watch.poster.set_visible(false);
                }
            }
        });
        self.start_over.set_visible(self.resume_at.get() > 0.0);
        self.set_inhibit(!self.paused.get());
        self.show_controls();
        let video = self.video.borrow().clone();
        if let Some(video) = video {
            (self.hooks.started)(&video);
        }
    }

    fn on_failed(self: &Rc<Self>, code: i32) {
        if self.in_pip() {
            // Bring the picture back so the problem can be seen and acted on.
            self.leave_pip(true);
        }
        let reason = self.last_error.borrow().clone();
        let reason = if reason.is_empty() { mpv::error_string(code) } else { reason };
        match util::explain_ytdlp(&reason, tools::sign_in().as_ref()) {
            Some(explained) => {
                self.show_problem(&explained.title, &explained.detail);
                // Another mpv window would meet the same refusal; a sign-in gets past it.
                self.elsewhere.set_visible(false);
                self.choose_browser.set_visible(true);
            }
            None => {
                self.show_problem(
                    "This video won't play",
                    &format!("{reason}\n\nUpdating yt-dlp usually fixes this."),
                );
                self.update_button.set_visible(true);
            }
        }
    }

    /// An update of yt-dlp finished; if it changed anything, a video that
    /// couldn't play tries again.
    pub fn yt_dlp_update_finished(self: &Rc<Self>, changed: bool) {
        self.update_button.set_sensitive(true);
        self.update_button.set_label("Update yt-dlp");
        if changed && self.shown.get() && self.problem.is_visible() {
            perf::mark("yt-dlp changed: trying the video again");
            self.retry();
        }
    }

    fn show_problem(&self, title: &str, detail: &str) {
        self.set_busy(false);
        self.problem_title.set_label(title);
        self.problem_detail.set_label(detail);
        self.elsewhere.set_visible(true);
        self.choose_browser.set_visible(false);
        self.update_button.set_visible(false);
        self.problem.set_visible(true);
        self.top.set_reveal_child(true);
        self.bottom.set_reveal_child(false);
        self.stage.set_cursor(None);
    }

    /// A different browser sign-in (or none) was chosen: a video on screen
    /// that couldn't play tries again with it.
    pub fn sign_in_changed(self: &Rc<Self>) {
        if self.shown.get() && self.problem.is_visible() {
            self.retry();
        }
    }

    fn retry(self: &Rc<Self>) {
        let video = self.video.borrow().clone();
        if let Some(video) = video {
            let resume = Some(self.resume_at.get()).filter(|t| *t > 0.0);
            self.open(&video, resume);
        }
    }

    fn pop_out(self: &Rc<Self>) {
        let video = self.video.borrow().clone();
        if let Some(video) = video {
            (self.hooks.pop_out)(&video, self.position.get());
        }
    }

    // ── Picture in picture ─────────────────────────────────────────────

    /// Moves the picture to a small window and returns to the feed; the video
    /// keeps playing.
    fn enter_pip(self: &Rc<Self>) {
        if self.in_pip() || !self.started.get() {
            return;
        }
        self.set_fullscreen(false);
        let app = self.window.borrow().upgrade().and_then(|w| w.application());
        let pip = self.build_pip(app);
        // Turn video off before its renderer goes away, so the switch is clean.
        self.video_off();
        self.main.release();
        self.main.area.set_visible(false);
        *self.pip.borrow_mut() = Some(pip.clone());
        self.reattach.set(true);
        let _ = self.page.activate_action("navigation.pop", None);
        pip.window.present();
        self.begin();
        self.update_play_button();
    }

    /// Closes the small window: back to the full player, or stop playing.
    fn leave_pip(self: &Rc<Self>, to_player: bool) {
        if !to_player {
            self.stop();
            return;
        }
        let pip = self.pip.borrow_mut().take();
        let Some(pip) = pip else { return };
        self.video_off();
        pip.surface.release();
        pip.window.destroy();
        self.reattach.set(true);
        // The page slides back in; `shown` puts the picture back on it.
        let _ = self.page.activate_action("navigation.push", Some(&"watch".to_variant()));
    }

    fn build_pip(self: &Rc<Self>, app: Option<gtk::Application>) -> Rc<Pip> {
        let surface = Surface::new("pip-video");
        // Dragging anywhere on the picture moves the window.
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&surface.area));

        let shade = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shade.add_css_class("pip-shade");
        shade.set_can_target(false);
        let back = icon_button("view-fullscreen-symbolic", "Back to the player (P)");
        let close = icon_button("window-close-symbolic", "Close");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("pip-bar");
        bar.set_valign(gtk::Align::Start);
        let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        gap.set_hexpand(true);
        bar.append(&back);
        bar.append(&gap);
        bar.append(&close);
        let play = icon_button("media-playback-pause-symbolic", "Pause (Space)");
        play.add_css_class("pip-play");
        play.set_halign(gtk::Align::Center);

        let layers = gtk::Overlay::new();
        layers.set_child(Some(&handle));
        layers.add_overlay(&shade);
        layers.add_overlay(&bar);
        layers.add_overlay(&play);

        let window = adw::Window::builder()
            .title("Picture in picture")
            .default_width(480)
            .default_height(270)
            .width_request(240)
            .height_request(135)
            .content(&layers)
            .build();
        window.add_css_class("pip");
        if let Some(app) = app {
            window.set_application(Some(&app));
        }

        let pip = Rc::new(Pip {
            window,
            surface,
            buttons: vec![bar.clone().upcast(), play.clone().upcast()],
            play,
            hide_source: RefCell::default(),
        });

        let weak = Rc::downgrade(self);
        back.connect_clicked(move |_| {
            if let Some(watch) = weak.upgrade() {
                watch.leave_pip(true);
            }
        });
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(watch) = weak.upgrade() {
                watch.leave_pip(false);
            }
        });
        let weak = Rc::downgrade(self);
        pip.play.connect_clicked(move |_| {
            if let Some(watch) = weak.upgrade() {
                watch.toggle_pause();
            }
        });
        let weak = Rc::downgrade(self);
        pip.window.connect_close_request(move |_| {
            if let Some(watch) = weak.upgrade() {
                watch.leave_pip(false);
            }
            glib::Propagation::Stop
        });

        // Controls show while the pointer is over the window.
        let hover = gtk::EventControllerMotion::new();
        let weak_pip = Rc::downgrade(&pip);
        hover.connect_motion(move |_, _, _| {
            if let Some(pip) = weak_pip.upgrade() {
                show_pip_controls(&pip);
            }
        });
        let weak_pip = Rc::downgrade(&pip);
        hover.connect_leave(move |_| {
            if let Some(pip) = weak_pip.upgrade() {
                set_pip_controls(&pip, false);
            }
        });
        layers.add_controller(hover);

        // Double-click the picture to go back to the full player.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |gesture, n_press, _, _| {
            if n_press == 2 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(watch) = weak.upgrade() {
                    watch.leave_pip(true);
                }
            }
        });
        handle.add_controller(click);

        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(watch) = weak.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::space | gdk::Key::k | gdk::Key::K => watch.toggle_pause(),
                gdk::Key::p | gdk::Key::P | gdk::Key::f | gdk::Key::F => watch.leave_pip(true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        pip.window.add_controller(keys);
        set_pip_controls(&pip, false);
        pip
    }

    /// The quality new videos start at, as set in Preferences.
    pub fn set_preferred_quality(&self, quality: Option<u32>) {
        self.quality.set(quality);
        self.quality_action.set_state(&quality_key(quality).to_variant());
    }

    /// A quality picked in the player: remembered, and applied to this video
    /// right away by reloading it from the current moment.
    fn choose_quality(self: &Rc<Self>, quality: Option<u32>) {
        self.quality.set(quality);
        (self.hooks.quality_changed)(quality);
        let video = self.video.borrow().clone();
        let Some(video) = video else { return };
        if !self.started.get() {
            return;
        }
        self.resume_at.set(self.position.get());
        *self.pending.borrow_mut() = Some(video.watch_url());
        self.set_busy(true);
        self.load_pending();
    }

    /// yt-dlp's format choice for the current quality setting.
    fn format(&self) -> String {
        let height = self.quality.get().unwrap_or_else(|| screen_height(self.window.borrow().upgrade().as_ref()));
        tools::ytdl_format(height)
    }

    fn video_off(&self) {
        if let Some(mpv) = self.mpv() {
            mpv.set_string("vid", "no");
        }
    }

    // ── For the desktop's media controls ───────────────────────────────

    pub fn play_pause(&self) {
        self.toggle_pause();
    }

    pub fn play(&self) {
        if self.ended.get() {
            self.toggle_pause();
        } else if let Some(mpv) = self.mpv() {
            mpv.set_flag("pause", false);
        }
    }

    pub fn pause(&self) {
        if let Some(mpv) = self.mpv() {
            mpv.set_flag("pause", true);
        }
    }

    /// Stops and leaves the player, whichever form it's in.
    pub fn close(self: &Rc<Self>) {
        if self.in_pip() {
            self.leave_pip(false);
        } else if self.shown.get() {
            let _ = self.page.activate_action("navigation.pop", None);
        }
    }

    pub fn position(&self) -> f64 {
        self.position.get()
    }

    pub fn set_autoplay(&self, on: bool) {
        self.autoplay.set(on);
        if !on {
            self.cancel_autoplay();
        }
    }

    fn status(&self) -> Status {
        if !self.open.get() {
            Status::Stopped
        } else if self.paused.get() || self.ended.get() {
            Status::Paused
        } else {
            Status::Playing
        }
    }

    /// Tells the window (and through it the media controls) how playback
    /// is going.
    fn notify_playback(&self) {
        let video = self.video.borrow().clone();
        let playback = Playback {
            status: self.status(),
            duration: self.duration.get(),
            speed: SPEEDS[self.speed_index.get()],
            volume: self.volume.value(),
        };
        (self.hooks.playback)(video.as_ref().filter(|_| self.open.get()), playback);
    }

    // ── Up next ────────────────────────────────────────────────────────

    /// The video ended: with autoplay on, offer the next one, with a few
    /// seconds to say no. In picture in picture it simply plays on.
    fn on_ended(self: &Rc<Self>) {
        if !self.autoplay.get() {
            return;
        }
        let Some(next) = (self.hooks.next_video)() else { return };
        if self.in_pip() {
            (self.hooks.play_next)();
            return;
        }
        self.up_next_title.set_label(&next.title);
        self.up_next_channel.set_label(&next.channel_name);
        self.up_next_thumb.set_paintable(None::<&gdk::Paintable>);
        let id = next.id.clone();
        // Noted before the thumbnail is asked for: one already in memory
        // arrives at once.
        *self.up_next_video.borrow_mut() = Some(next);
        let weak = Rc::downgrade(self);
        (self.hooks.thumbnail)(
            &id.clone(),
            Box::new(move |texture| {
                if let Some(watch) = weak.upgrade() {
                    if watch.up_next_video.borrow().as_ref().is_some_and(|v| v.id == id) {
                        watch.up_next_thumb.set_paintable(Some(texture));
                    }
                }
            }),
        );
        self.countdown_left.set(AUTOPLAY_SECONDS);
        self.show_countdown();
        self.up_next.set_visible(true);
        if let Some(source) = self.countdown_source.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_seconds_local(1, move || {
            let Some(watch) = weak.upgrade() else { return glib::ControlFlow::Break };
            let left = watch.countdown_left.get().saturating_sub(1);
            watch.countdown_left.set(left);
            if left == 0 {
                watch.countdown_source.borrow_mut().take();
                watch.play_next_now();
                return glib::ControlFlow::Break;
            }
            watch.show_countdown();
            glib::ControlFlow::Continue
        });
        *self.countdown_source.borrow_mut() = Some(source);
    }

    fn show_countdown(&self) {
        self.up_next_count.set_label(&format!("Playing in {} s", self.countdown_left.get()));
    }

    fn cancel_autoplay(&self) {
        if let Some(source) = self.countdown_source.borrow_mut().take() {
            source.remove();
        }
        self.up_next_video.borrow_mut().take();
        self.up_next.set_visible(false);
    }

    fn play_next_now(self: &Rc<Self>) {
        self.cancel_autoplay();
        (self.hooks.play_next)();
    }

    // ── Chapters ───────────────────────────────────────────────────────

    fn read_chapters(&self, mpv: &Mpv) {
        let count = mpv.string("chapter-list/count").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(500);
        let chapters: Vec<(f64, String)> = (0..count)
            .map(|i| {
                let time = mpv.double(&format!("chapter-list/{i}/time")).unwrap_or(0.0);
                let title = mpv.string(&format!("chapter-list/{i}/title")).unwrap_or_default();
                (time, title)
            })
            .collect();
        *self.chapters.borrow_mut() = chapters;
        self.apply_chapter_marks();
        self.update_chapter_label();
    }

    /// Marks each chapter's start on the seek bar.
    fn apply_chapter_marks(&self) {
        self.seek.clear_marks();
        let duration = self.duration.get();
        if duration <= 0.0 {
            return;
        }
        for (time, _) in self.chapters.borrow().iter() {
            if *time > 0.5 && *time < duration {
                self.seek.add_mark(*time, gtk::PositionType::Bottom, None);
            }
        }
    }

    fn update_chapter_label(&self) {
        let title = self
            .mpv()
            .and_then(|mpv| mpv.string("chapter"))
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| self.chapters.borrow().get(index).map(|(_, title)| title.clone()))
            .filter(|title| !title.is_empty());
        match title {
            Some(title) => {
                self.chapter.set_label(&title);
                self.chapter.set_visible(true);
            }
            None => self.chapter.set_visible(false),
        }
    }

    /// To the next (1) or previous (-1) chapter.
    fn step_chapter(&self, step: i32) {
        if self.chapters.borrow().is_empty() {
            return;
        }
        if let Some(mpv) = self.mpv() {
            mpv.command_async(&["add", "chapter", &step.to_string()]);
        }
    }

    // ── Watch later ────────────────────────────────────────────────────

    fn toggle_saved(&self) {
        let video = self.video.borrow().clone();
        if let Some(video) = video {
            let saved = (self.hooks.toggle_saved)(&video);
            self.show_saved(saved);
        }
    }

    fn show_saved(&self, saved: bool) {
        if saved {
            self.save.add_css_class("active");
        } else {
            self.save.remove_css_class("active");
        }
        self.save.set_tooltip_text(Some(if saved { "Saved for later (S)" } else { "Save for later (S)" }));
    }

    // ── Subtitles ──────────────────────────────────────────────────────

    pub fn toggle_subtitles(&self) {
        let on = !self.subtitles.get();
        self.subtitles.set(on);
        if on {
            self.subtitles_button.add_css_class("active");
        } else {
            self.subtitles_button.remove_css_class("active");
        }
        (self.hooks.subtitles_changed)(on);
        if let Some(mpv) = self.mpv() {
            if on {
                self.select_subtitles(&mpv);
            } else {
                mpv.set_string("sid", "no");
            }
        }
    }

    /// Shows the subtitle track in the viewer's language, or English, or
    /// whichever comes first.
    fn select_subtitles(&self, mpv: &Mpv) {
        let wanted = tools::preferred_language();
        let count = mpv.string("track-list/count").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(200);
        let mut best: Option<(u8, String)> = None;
        for i in 0..count {
            if mpv.string(&format!("track-list/{i}/type")).as_deref() != Some("sub") {
                continue;
            }
            let Some(id) = mpv.string(&format!("track-list/{i}/id")) else { continue };
            let lang = mpv.string(&format!("track-list/{i}/lang")).unwrap_or_default().to_lowercase();
            let score = if lang.starts_with(&wanted) {
                2
            } else if lang.starts_with("en") {
                1
            } else {
                0
            };
            if best.as_ref().is_none_or(|(so_far, _)| score > *so_far) {
                best = Some((score, id));
            }
        }
        if let Some((_, id)) = best {
            mpv.set_string("sid", &id);
        }
    }

    // ── Controls ───────────────────────────────────────────────────────

    fn toggle_pause(&self) {
        let Some(mpv) = self.mpv() else { return };
        if self.ended.get() {
            self.cancel_autoplay();
            mpv.command_async(&["seek", "0", "absolute"]);
            mpv.set_flag("pause", false);
        } else {
            mpv.set_flag("pause", !self.paused.get());
        }
    }

    pub fn seek_to(&self, seconds: f64) {
        let Some(mpv) = self.mpv() else { return };
        self.cancel_autoplay();
        // Hold off position updates briefly so the bar doesn't jump back while seeking.
        self.seek_hold_until.set(glib::monotonic_time() + 500_000);
        self.seek_target.set(seconds);
        self.seek.set_value(seconds);
        mpv.command_async(&["seek", &format!("{seconds:.3}"), "absolute"]);
        (self.hooks.seeked)(seconds);
    }

    fn start_over(&self) {
        self.start_over.set_visible(false);
        self.resume_at.set(0.0);
        self.seek_to(0.0);
    }

    pub fn seek_by(&self, delta: f64) {
        let end = self.duration.get().max(0.0);
        let seeking = glib::monotonic_time() < self.seek_hold_until.get();
        let from = if seeking { self.seek_target.get() } else { self.position.get() };
        self.seek_to((from + delta).clamp(0.0, end));
    }

    pub fn set_volume(&self, volume: f64) {
        let Some(mpv) = self.mpv() else { return };
        mpv.set_double("volume", volume.clamp(0.0, 100.0));
        if self.muted.get() {
            mpv.set_flag("mute", false);
        }
    }

    fn toggle_mute(&self) {
        if let Some(mpv) = self.mpv() {
            mpv.set_flag("mute", !self.muted.get());
        }
    }

    fn next_speed(&self) {
        let index = (self.speed_index.get() + 1) % SPEEDS.len();
        self.speed_index.set(index);
        self.speed.set_label(&speed_label(SPEEDS[index]));
        if let Some(mpv) = self.mpv() {
            mpv.set_double("speed", SPEEDS[index]);
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.window.borrow().upgrade().is_some_and(|w| w.is_fullscreen())
    }

    fn set_fullscreen(&self, on: bool) {
        if let Some(window) = self.window.borrow().upgrade() {
            if on {
                window.fullscreen();
            } else if window.is_fullscreen() {
                window.unfullscreen();
            }
        }
    }

    fn toggle_fullscreen(&self) {
        self.set_fullscreen(!self.is_fullscreen());
    }

    fn sync_fullscreen(&self, on: bool) {
        let (icon, tip) = if on {
            ("view-restore-symbolic", "Leave full screen (F)")
        } else {
            ("view-fullscreen-symbolic", "Full screen (F)")
        };
        self.fullscreen.set_icon_name(&icons::get(icon));
        self.fullscreen.set_tooltip_text(Some(tip));
    }

    /// Keeps the screen from dimming or locking while a video plays.
    fn set_inhibit(&self, on: bool) {
        let Some(window) = self.window.borrow().upgrade() else { return };
        let Some(app) = window.application() else { return };
        let cookie = self.inhibit_cookie.get();
        if on && cookie == 0 {
            let cookie = app.inhibit(Some(&window), gtk::ApplicationInhibitFlags::IDLE, Some("Playing a video"));
            self.inhibit_cookie.set(cookie);
        } else if !on && cookie != 0 {
            app.uninhibit(cookie);
            self.inhibit_cookie.set(0);
        }
    }

    fn show_controls(self: &Rc<Self>) {
        self.top.set_reveal_child(true);
        if !self.problem.is_visible() {
            self.bottom.set_reveal_child(true);
        }
        self.refresh_progress(true);
        self.stage.set_cursor(None);
        if let Some(source) = self.hide_source.borrow_mut().take() {
            source.remove();
        }
        if !self.started.get() || self.paused.get() || self.ended.get() || self.problem.is_visible() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(HIDE_CONTROLS_AFTER, move || {
            if let Some(watch) = weak.upgrade() {
                watch.hide_source.borrow_mut().take();
                watch.hide_controls();
            }
        });
        *self.hide_source.borrow_mut() = Some(source);
    }

    fn hide_controls(self: &Rc<Self>) {
        if self.over_controls.get() {
            self.show_controls();
            return;
        }
        if !self.started.get() || self.paused.get() || self.ended.get() {
            return;
        }
        self.top.set_reveal_child(false);
        self.bottom.set_reveal_child(false);
        self.stage.set_cursor_from_name(Some("none"));
    }

    fn set_busy(&self, busy: bool) {
        self.spinner.set_visible(busy);
        self.spinner.set_spinning(busy);
    }

    /// Updates the seek bar and clock, a few times a second at most and only
    /// while they're on screen: changing them reflows the controls, which is
    /// work the video doesn't need on every frame.
    fn refresh_progress(&self, force: bool) {
        if !force && !self.bottom.reveals_child() {
            return;
        }
        let now = glib::monotonic_time();
        if !force && now - self.last_progress.get() < PROGRESS_INTERVAL_US {
            return;
        }
        self.last_progress.set(now);
        if now > self.seek_hold_until.get() {
            self.seek.set_value(self.position.get());
        }
        self.seek.set_fill_level(self.cache_time.get());
        let clock = |seconds: f64| util::duration(seconds.max(0.0) as u32);
        let (position, duration) = (self.position.get(), self.duration.get());
        let text = if duration > 0.0 {
            format!("{} / {}", clock(position), clock(duration))
        } else {
            clock(position)
        };
        if *self.time_text.borrow() != text {
            self.time.set_label(&text);
            *self.time_text.borrow_mut() = text;
        }
    }

    fn update_play_button(&self) {
        let (icon, tip) = if self.ended.get() {
            ("media-playlist-repeat-symbolic", "Watch again (Space)")
        } else if self.paused.get() {
            ("media-playback-start-symbolic", "Play (Space)")
        } else {
            ("media-playback-pause-symbolic", "Pause (Space)")
        };
        self.play.set_icon_name(&icons::get(icon));
        self.play.set_tooltip_text(Some(tip));
        if let Some(pip) = self.pip.borrow().as_ref() {
            pip.play.set_icon_name(&icons::get(icon));
            pip.play.set_tooltip_text(Some(tip));
        }
    }

    fn update_mute_button(&self) {
        let silent = self.muted.get() || self.volume.value() <= 0.0;
        let icon = if silent { "audio-volume-muted-symbolic" } else { "audio-volume-high-symbolic" };
        self.mute.set_icon_name(&icons::get(icon));
        self.mute
            .set_tooltip_text(Some(if self.muted.get() { "Unmute (M)" } else { "Mute (M)" }));
    }
}

fn set_pip_controls(pip: &Pip, on: bool) {
    if on {
        pip.window.add_css_class("show-controls");
    } else {
        pip.window.remove_css_class("show-controls");
    }
    // Hidden controls mustn't catch clicks meant for dragging the window.
    for widget in &pip.buttons {
        widget.set_can_target(on);
    }
}

fn show_pip_controls(pip: &Rc<Pip>) {
    set_pip_controls(pip, true);
    if let Some(source) = pip.hide_source.borrow_mut().take() {
        source.remove();
    }
    let weak = Rc::downgrade(pip);
    let source = glib::timeout_add_local_once(HIDE_PIP_CONTROLS_AFTER, move || {
        if let Some(pip) = weak.upgrade() {
            pip.hide_source.borrow_mut().take();
            set_pip_controls(&pip, false);
        }
    });
    *pip.hide_source.borrow_mut() = Some(source);
}

/// Shrinks a thumbnail to 32×18, crops YouTube's letterbox bars, and keeps
/// premultiplied colors, so the GPU can stretch it into a soft backdrop.
fn soften(texture: &gdk::Texture) -> Option<gdk::Texture> {
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    if width < 32 || height < 18 {
        return None;
    }
    let stride = width * 4;
    let mut pixels = vec![0u8; stride * height];
    texture.download(&mut pixels, stride);
    let (top, rows) = if (width as f64 / height as f64 - 4.0 / 3.0).abs() < 0.02 {
        let rows = width * 9 / 16;
        ((height - rows) / 2, rows)
    } else {
        (0, height)
    };
    const OUT_W: usize = 32;
    const OUT_H: usize = 18;
    let mut out = vec![0u8; OUT_W * OUT_H * 4];
    for oy in 0..OUT_H {
        let (y0, y1) = (top + oy * rows / OUT_H, top + ((oy + 1) * rows / OUT_H).max(oy * rows / OUT_H + 1));
        for ox in 0..OUT_W {
            let (x0, x1) = (ox * width / OUT_W, ((ox + 1) * width / OUT_W).max(ox * width / OUT_W + 1));
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in y0..y1.min(height) {
                for x in x0..x1.min(width) {
                    let i = y * stride + x * 4;
                    for channel in 0..4 {
                        sum[channel] += u32::from(pixels[i + channel]);
                    }
                    count += 1;
                }
            }
            let o = (oy * OUT_W + ox) * 4;
            for channel in 0..4 {
                out[o + channel] = (sum[channel] / count.max(1)) as u8;
            }
        }
    }
    let bytes = glib::Bytes::from_owned(out);
    let soft = gdk::MemoryTexture::new(OUT_W as i32, OUT_H as i32, gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, OUT_W * 4);
    Some(soft.upcast())
}

/// "Match my screen": the tallest common video size the monitor showing
/// `window` displays pixel for pixel, and never less than 1080p.
pub fn screen_height(window: Option<&gtk::Window>) -> u32 {
    let physical = window
        .and_then(|window| {
            let surface = window.surface()?;
            let monitor = WidgetExt::display(window).monitor_at_surface(&surface)?;
            // The surface knows fractional scales (125%, 150%); the monitor's
            // scale factor rounds them up, which would overstate the pixels.
            Some((f64::from(monitor.geometry().height()) * surface.scale()).round() as i32)
        })
        .unwrap_or(1080);
    match physical {
        h if h >= 2000 => 2160,
        h if h >= 1350 => 1440,
        _ => 1080,
    }
}

/// Options for mpv's YouTube hook: go straight to yt-dlp instead of first
/// fetching the YouTube page itself, and use the yt-dlp Matinee found, since
/// launchers can leave ~/.local/bin off PATH. Set again before each video:
/// the hook looks yt-dlp up afresh when these change (an update can put a
/// new copy in ~/.local/bin).
fn ytdl_script_opts() -> String {
    let mut script_opts = vec!["ytdl_hook-try_ytdl_first=yes".to_string()];
    if let Some(path) = tools::find_yt_dlp() {
        script_opts.push(format!("ytdl_hook-ytdl_path={}", path.display()));
    }
    script_opts.join(",")
}

/// Zero-copy hardware decoding, opt-in until it's proven on more drivers.
fn interop() -> bool {
    std::env::var_os("MATINEE_HWDEC_INTEROP").is_some_and(|v| v == "1")
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(&icons::get(icon));
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button
}

const QUALITY_CHOICES: [(&str, &str); 6] = [
    ("auto", "Match my screen"),
    ("2160", "4K (2160p)"),
    ("1440", "1440p"),
    ("1080", "1080p"),
    ("720", "720p"),
    ("480", "480p"),
];

fn quality_key(quality: Option<u32>) -> String {
    quality.map_or_else(|| "auto".to_string(), |height| height.to_string())
}

/// What's actually playing, shown on the quality button.
fn height_label(height: f64) -> String {
    match height.round() as u32 {
        0 => "Auto".to_string(),
        h if h >= 2100 => "4K".to_string(),
        h => format!("{h}p"),
    }
}

fn speed_label(speed: f64) -> String {
    let text = format!("{speed:.2}");
    format!("{}×", text.trim_end_matches('0').trim_end_matches('.'))
}
