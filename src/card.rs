//! A recyclable video card. The feed keeps only cards near the screen alive
//! and re-binds them to other videos as you scroll, so a feed of thousands
//! costs about the same as a feed of twenty.
//!
//! Motion is kept cheap: the thumbnail fades and zooms inside its own
//! paintable (a repaint, never a relayout), and the entrance (a short rise
//! and fade) only plays when a feed first appears.

use crate::model::Video;
use crate::thumb::{ThumbPaintable, HOVER_ZOOM};
use crate::{icons, util};
use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

thread_local! {
    static SCROLLING_UNTIL: Cell<i64> = const { Cell::new(0) };
}

/// Called while the feed scrolls: cards gliding under a still pointer
/// shouldn't each start a hover animation.
pub fn note_scroll() {
    SCROLLING_UNTIL.with(|until| until.set(glib::monotonic_time() + 200_000));
}

pub struct CardOpts {
    pub is_new: bool,
    pub watched: bool,
    pub show_channel: bool,
    pub subscribed: bool,
    pub progress: f64,
}

struct Bound {
    id: String,
    channel_id: String,
    channel_name: String,
    watched: bool,
    subscribed: bool,
}

pub struct Card {
    pub root: gtk::Button,
    body: gtk::Box,
    thumb: ThumbPaintable,
    title: gtk::Label,
    channel: gtk::Label,
    views: gtk::Label,
    when: gtk::Label,
    length: gtk::Label,
    new_chip: gtk::Label,
    watched_chip: gtk::Label,
    bound: RefCell<Option<Bound>>,
    thumb_request: RefCell<Option<(String, u64)>>,
    /// A thumbnail to fetch once the card is near the screen.
    thumb_wanted: RefCell<Option<String>>,
    hover_animation: RefCell<Option<adw::TimedAnimation>>,
    arrival_animation: RefCell<Option<adw::TimedAnimation>>,
    reveal_source: RefCell<Option<glib::SourceId>>,
    on_focus: RefCell<Option<Box<dyn Fn()>>>,
    hovered: Cell<bool>,
    /// The card's place in the feed, for reading ahead.
    position: Cell<u32>,
}

impl Card {
    pub fn new(min_width: i32) -> Rc<Self> {
        let thumb = ThumbPaintable::default();
        let picture = gtk::Picture::for_paintable(&thumb);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Fill);

        let frame = gtk::Overlay::new();
        frame.add_css_class("thumb-frame");
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_child(Some(&picture));

        let glyph = gtk::Image::from_icon_name(&icons::get("media-playback-start-symbolic"));
        glyph.set_pixel_size(20);
        glyph.add_css_class("play-glyph");
        glyph.set_halign(gtk::Align::Center);
        glyph.set_valign(gtk::Align::Center);
        glyph.set_can_target(false);
        frame.add_overlay(&glyph);

        let when = chip(&frame, "when", gtk::Align::Start, gtk::Align::End);
        let length = chip(&frame, "length", gtk::Align::End, gtk::Align::End);
        let new_chip = chip(&frame, "new", gtk::Align::Start, gtk::Align::Start);
        new_chip.set_label("New");
        let watched_chip = chip(&frame, "watched-chip", gtk::Align::End, gtk::Align::Start);
        watched_chip.set_label("Watched");

        let title = gtk::Label::new(None);
        title.add_css_class("card-title");
        title.set_xalign(0.0);
        title.set_yalign(0.0);
        title.set_wrap(true);
        title.set_wrap_mode(pango::WrapMode::WordChar);
        title.set_lines(2);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_max_width_chars(32);

        let channel = gtk::Label::new(None);
        channel.add_css_class("card-channel");
        channel.set_xalign(0.0);
        channel.set_hexpand(true);
        channel.set_ellipsize(pango::EllipsizeMode::End);
        channel.set_max_width_chars(18);
        let views = gtk::Label::new(None);
        views.add_css_class("card-views");
        views.set_xalign(1.0);
        let meta = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        meta.add_css_class("card-meta");
        meta.append(&channel);
        meta.append(&views);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
        text.add_css_class("card-text");
        text.append(&title);
        text.append(&meta);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 11);
        body.append(&frame);
        body.append(&text);

        let root = gtk::Button::new();
        root.add_css_class("video-card");
        root.set_child(Some(&body));
        root.set_focus_on_click(false);
        root.set_action_name(Some("win.play"));
        root.set_size_request(min_width, -1);
        root.set_valign(gtk::Align::Start);

        let card = Rc::new(Self {
            root,
            body,
            thumb,
            title,
            channel,
            views,
            when,
            length,
            new_chip,
            watched_chip,
            bound: RefCell::default(),
            thumb_request: RefCell::default(),
            thumb_wanted: RefCell::default(),
            hover_animation: RefCell::default(),
            arrival_animation: RefCell::default(),
            reveal_source: RefCell::default(),
            on_focus: RefCell::default(),
            hovered: Cell::new(false),
            position: Cell::new(0),
        });
        card.connect_controllers();
        card
    }

    fn connect_controllers(self: &Rc<Self>) {
        let menu_at = |card: &Rc<Card>, x: f64, y: f64| {
            if card.bound.borrow().is_some() {
                card.popup_menu(x, y);
            }
        };
        let weak = Rc::downgrade(self);
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(card) = weak.upgrade() {
                menu_at(&card, x, y);
            }
        });
        self.root.add_controller(right_click);

        let weak = Rc::downgrade(self);
        let long_press = gtk::GestureLongPress::new();
        long_press.set_touch_only(true);
        long_press.connect_pressed(move |gesture, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(card) = weak.upgrade() {
                menu_at(&card, x, y);
            }
        });
        self.root.add_controller(long_press);

        let hover = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        hover.connect_enter(move |_, _, _| {
            if let Some(card) = weak.upgrade() {
                card.set_hovered(true);
            }
        });
        let weak = Rc::downgrade(self);
        hover.connect_leave(move |_| {
            if let Some(card) = weak.upgrade() {
                card.set_hovered(false);
            }
        });
        self.root.add_controller(hover);

        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(card) = weak.upgrade() {
                if let Some(callback) = card.on_focus.borrow().as_ref() {
                    callback();
                }
            }
        });
        self.root.add_controller(focus);
    }

    /// Shows `video` on this card.
    pub fn bind(&self, video: &Video, opts: CardOpts, now: i64) {
        self.root.set_action_target_value(Some(&video.id.to_variant()));
        self.root.set_sensitive(true);
        self.root.set_can_target(true);
        self.root.set_opacity(1.0);
        self.root.remove_css_class("enter");

        self.title.set_label(&video.title);
        // Two lines hold about 64 characters; only longer titles need a tooltip.
        self.root
            .set_tooltip_text((video.title.chars().count() > 64).then_some(video.title.as_str()));
        let show_channel = opts.show_channel && !video.channel_name.is_empty();
        self.channel.set_visible(show_channel);
        self.channel.set_label(&video.channel_name);
        match video.views {
            Some(n) => {
                self.views.set_label(&util::views(n));
                self.views.set_visible(true);
            }
            None => self.views.set_visible(false),
        }
        self.views.set_xalign(if show_channel { 1.0 } else { 0.0 });
        set_chip(&self.when, video.published.map(|ts| util::time_ago(ts, now)));
        set_chip(&self.length, video.duration.map(util::duration));
        self.new_chip.set_visible(opts.is_new && !opts.watched);
        self.watched_chip.set_visible(opts.watched);
        if opts.watched {
            self.root.add_css_class("watched");
        } else {
            self.root.remove_css_class("watched");
        }

        self.stop_animations();
        self.thumb.set_texture(None);
        self.thumb.set_reveal(1.0);
        self.thumb.set_hover(if self.hovered.get() { HOVER_ZOOM } else { 1.0 });
        self.thumb.set_dim(if opts.watched { 0.45 } else { 0.0 });
        self.thumb.set_progress(opts.progress);
        self.root.update_property(&[gtk::accessible::Property::Label(&format!(
            "{}, {}",
            video.title, video.channel_name
        ))]);

        *self.bound.borrow_mut() = Some(Bound {
            id: video.id.clone(),
            channel_id: video.channel_id.clone(),
            channel_name: video.channel_name.clone(),
            watched: opts.watched,
            subscribed: opts.subscribed,
        });
    }

    pub fn unbind(&self) {
        self.bound.borrow_mut().take();
        self.thumb_wanted.borrow_mut().take();
        self.on_focus.borrow_mut().take();
        self.stop_animations();
        self.thumb.set_texture(None);
        self.root.remove_css_class("enter");
        self.root.set_opacity(1.0);
    }

    pub fn set_min_width(&self, width: i32) {
        self.root.set_size_request(width, -1);
    }

    pub fn set_position(&self, position: u32) {
        self.position.set(position);
    }

    pub fn position(&self) -> u32 {
        self.position.get()
    }

    pub fn bound_id(&self) -> Option<String> {
        self.bound.borrow().as_ref().map(|b| b.id.clone())
    }

    pub fn set_thumb_request(&self, request: Option<(String, u64)>) {
        *self.thumb_request.borrow_mut() = request;
    }

    pub fn take_thumb_request(&self) -> Option<(String, u64)> {
        self.thumb_request.borrow_mut().take()
    }

    /// Asks for `id`'s thumbnail as soon as the card comes near the screen.
    pub fn want_thumb(&self, id: &str) {
        *self.thumb_wanted.borrow_mut() = Some(id.to_string());
    }

    pub fn take_wanted_thumb(&self) -> Option<String> {
        self.thumb_wanted.borrow_mut().take()
    }

    pub fn has_wanted_thumb(&self) -> bool {
        self.thumb_wanted.borrow().is_some()
    }

    /// Lets go of the image of a card far off screen, to keep memory flat on
    /// long scrolls. Returns the pending request to cancel, if any; the image
    /// is asked for again when the card comes back.
    pub fn forget_thumb(&self) -> Option<(String, u64)> {
        let id = self.bound_id()?;
        let request = self.thumb_request.borrow_mut().take();
        if request.is_some() || self.thumb.has_texture() {
            self.thumb.set_texture(None);
            *self.thumb_wanted.borrow_mut() = Some(id);
        }
        request
    }

    pub fn set_on_focus(&self, callback: impl Fn() + 'static) {
        *self.on_focus.borrow_mut() = Some(Box::new(callback));
    }

    /// Puts the thumbnail in place. Images that had to be fetched fade up
    /// from the dark; ones already in memory appear at once, which keeps fast
    /// scrolling crisp.
    pub fn show_texture(&self, id: &str, texture: &gdk::Texture, fade: bool) {
        if self.bound_id().as_deref() != Some(id) {
            return;
        }
        self.thumb.set_texture(Some(texture));
        self.thumb_request.borrow_mut().take();
        if !fade || !adw::is_animations_enabled(&self.root) {
            self.thumb.set_reveal(1.0);
            return;
        }
        let thumb = self.thumb.clone();
        let target = adw::CallbackAnimationTarget::new(move |value| thumb.set_reveal(value));
        let animation = adw::TimedAnimation::new(&self.root, 0.0, 1.0, 650, target);
        animation.set_easing(adw::Easing::EaseOutCubic);
        animation.play();
        if let Some(old) = self.arrival_animation.replace(Some(animation)) {
            old.pause();
        }
    }

    /// Entrance for the first screen of a freshly shown feed.
    pub fn enter_after(self: &Rc<Self>, delay_ms: u64) {
        self.root.set_opacity(0.0);
        self.root.remove_css_class("enter");
        if let Some(source) = self.reveal_source.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
            if let Some(card) = weak.upgrade() {
                card.reveal_source.borrow_mut().take();
                card.root.set_opacity(1.0);
                card.root.add_css_class("enter");
            }
        });
        *self.reveal_source.borrow_mut() = Some(source);
    }

    pub fn set_watched(&self, watched: bool) {
        if let Some(bound) = self.bound.borrow_mut().as_mut() {
            bound.watched = watched;
        }
        self.watched_chip.set_visible(watched);
        self.thumb.set_dim(if watched { 0.45 } else { 0.0 });
        if watched {
            self.root.add_css_class("watched");
            self.new_chip.set_visible(false);
        } else {
            self.root.remove_css_class("watched");
        }
    }

    pub fn set_subscribed(&self, subscribed: bool) {
        if let Some(bound) = self.bound.borrow_mut().as_mut() {
            bound.subscribed = subscribed;
        }
    }

    pub fn set_progress(&self, fraction: f64) {
        self.thumb.set_progress(fraction);
    }

    fn set_hovered(&self, hovered: bool) {
        self.hovered.set(hovered);
        if self.bound.borrow().is_none() {
            return;
        }
        if hovered && SCROLLING_UNTIL.with(Cell::get) > glib::monotonic_time() {
            return;
        }
        let to = if hovered { HOVER_ZOOM } else { 1.0 };
        if !adw::is_animations_enabled(&self.root) {
            self.thumb.set_hover(to);
            return;
        }
        let thumb = self.thumb.clone();
        let target = adw::CallbackAnimationTarget::new(move |value| thumb.set_hover(value));
        let animation = adw::TimedAnimation::new(&self.root, self.thumb.hover(), to, 380, target);
        animation.set_easing(adw::Easing::EaseOutCubic);
        animation.play();
        if let Some(old) = self.hover_animation.replace(Some(animation)) {
            old.pause();
        }
    }

    fn stop_animations(&self) {
        for slot in [&self.hover_animation, &self.arrival_animation] {
            if let Some(animation) = slot.borrow_mut().take() {
                animation.pause();
            }
        }
        if let Some(source) = self.reveal_source.borrow_mut().take() {
            source.remove();
        }
    }

    fn menu(&self) -> Option<gio::Menu> {
        let bound = self.bound.borrow();
        let bound = bound.as_ref()?;
        let item = |label: &str, action: &str, target: &str| {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
            item
        };
        let menu = gio::Menu::new();

        let watch = gio::Menu::new();
        watch.append_item(&item("Play", "win.play", &bound.id));
        watch.append_item(&item("Play in separate window", "win.play-external", &bound.id));
        let toggle = if bound.watched { "Mark as unwatched" } else { "Mark as watched" };
        watch.append_item(&item(toggle, "win.toggle-watched", &bound.id));
        menu.append_section(None, &watch);

        let share = gio::Menu::new();
        share.append_item(&item("Copy link", "win.copy-link", &bound.id));
        share.append_item(&item("Open in browser", "win.open-browser", &bound.id));
        menu.append_section(None, &share);

        if !bound.channel_id.is_empty() {
            // Menu labels treat "_" as a mnemonic marker; double it to show it literally.
            let name = bound.channel_name.replace('_', "__");
            let channel = gio::Menu::new();
            if bound.subscribed {
                channel.append_item(&item(&format!("Go to {name}"), "win.show-channel", &bound.channel_id));
            } else {
                channel.append_item(&item(&format!("Subscribe to {name}"), "win.subscribe", &bound.id));
            }
            menu.append_section(None, &channel);
        }
        Some(menu)
    }

    fn popup_menu(&self, x: f64, y: f64) {
        let Some(menu) = self.menu() else { return };
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&self.body);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            // Unparent after the chosen item's action has run.
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    }
}

fn chip(frame: &gtk::Overlay, class: &str, h: gtk::Align, v: gtk::Align) -> gtk::Label {
    let chip = gtk::Label::new(None);
    chip.add_css_class("chip");
    chip.add_css_class(class);
    chip.set_halign(h);
    chip.set_valign(v);
    chip.set_can_target(false);
    chip.set_visible(false);
    frame.add_overlay(&chip);
    chip
}

fn set_chip(chip: &gtk::Label, text: Option<String>) {
    match text {
        Some(text) => {
            chip.set_label(&text);
            chip.set_visible(true);
        }
        None => chip.set_visible(false),
    }
}
