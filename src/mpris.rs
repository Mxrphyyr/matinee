//! The desktop's media controls and the keyboard's media keys, through MPRIS
//! (`org.mpris.MediaPlayer2`) on the session bus. Plasma's media player
//! widget shows what's playing, with the thumbnail, and its media-key
//! shortcuts drive whichever MPRIS player is active. Nothing here talks to
//! anything but the local desktop.

use gtk::glib;
use gtk::{gio, prelude::*};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

const BUS_NAME: &str = "org.mpris.MediaPlayer2.matinee";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

const INTROSPECTION: &str = r#"
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg direction="in" name="Offset" type="x"/></method>
    <method name="SetPosition"><arg direction="in" name="TrackId" type="o"/><arg direction="in" name="Position" type="x"/></method>
    <method name="OpenUri"><arg direction="in" name="Uri" type="s"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="LoopStatus" type="s" access="readwrite"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Shuffle" type="b" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Playing => "Playing",
            Status::Paused => "Paused",
            Status::Stopped => "Stopped",
        }
    }
}

/// What's playing, as the controls see it.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Now {
    pub status: Status,
    /// The video's id, or empty when nothing is loaded.
    pub id: String,
    pub title: String,
    pub channel: String,
    pub url: String,
    /// The thumbnail on disk, if it's there.
    pub art: Option<PathBuf>,
    /// Seconds; 0 while unknown.
    pub length: f64,
    /// 0 to 1.
    pub volume: f64,
    pub rate: f64,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

/// What the controls can ask of Matinee. Each runs on the UI thread.
pub struct Hooks {
    pub play_pause: Box<dyn Fn()>,
    pub play: Box<dyn Fn()>,
    pub pause: Box<dyn Fn()>,
    pub stop: Box<dyn Fn()>,
    pub next: Box<dyn Fn()>,
    pub previous: Box<dyn Fn()>,
    /// Seconds, relative.
    pub seek_by: Box<dyn Fn(f64)>,
    /// Seconds, absolute.
    pub seek_to: Box<dyn Fn(f64)>,
    /// 0 to 1.
    pub set_volume: Box<dyn Fn(f64)>,
    /// The current position in seconds.
    pub position: Box<dyn Fn() -> f64>,
    pub raise: Box<dyn Fn()>,
    pub quit: Box<dyn Fn()>,
}

pub struct Mpris {
    hooks: Hooks,
    now: RefCell<Now>,
    connection: RefCell<Option<gio::DBusConnection>>,
    owner: Cell<Option<gio::OwnerId>>,
    registrations: RefCell<Vec<gio::RegistrationId>>,
}

impl Mpris {
    /// Claims the player's name on the bus; the controls appear once it's granted.
    pub fn start(hooks: Hooks) -> Rc<Self> {
        let mpris = Rc::new(Self {
            hooks,
            now: RefCell::default(),
            connection: RefCell::default(),
            owner: Cell::new(None),
            registrations: RefCell::default(),
        });
        let weak = Rc::downgrade(&mpris);
        let owner = gio::bus_own_name(
            gio::BusType::Session,
            BUS_NAME,
            gio::BusNameOwnerFlags::NONE,
            move |connection, _| {
                if let Some(mpris) = weak.upgrade() {
                    mpris.register(connection);
                }
            },
            |_, _| {},
            |_, _| {},
        );
        mpris.owner.set(Some(owner));
        mpris
    }

    fn register(self: &Rc<Self>, connection: gio::DBusConnection) {
        let Ok(node) = gio::DBusNodeInfo::for_xml(INTROSPECTION) else { return };
        for name in [ROOT, PLAYER] {
            let Some(interface) = node.lookup_interface(name) else { continue };
            let calls = Rc::downgrade(self);
            let gets = Rc::downgrade(self);
            let sets = Rc::downgrade(self);
            let registration = connection
                .register_object(PATH, &interface)
                .method_call(move |_, _, _, interface, method, parameters, invocation| {
                    if let Some(mpris) = calls.upgrade() {
                        mpris.call(interface.unwrap_or(""), method, &parameters, invocation);
                    } else {
                        invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "Matinee is closing");
                    }
                })
                .property(move |_, _, _, interface, property| {
                    gets.upgrade().map_or_else(|| false.to_variant(), |mpris| mpris.get(interface, property))
                })
                .set_property(move |_, _, _, interface, property, value| {
                    sets.upgrade().is_some_and(|mpris| mpris.set(interface, property, &value))
                })
                .build();
            if let Ok(id) = registration {
                self.registrations.borrow_mut().push(id);
            }
        }
        *self.connection.borrow_mut() = Some(connection);
    }

    fn call(&self, interface: &str, method: &str, parameters: &glib::Variant, invocation: gio::DBusMethodInvocation) {
        let seconds = |index: usize| parameters.try_child_get::<i64>(index).ok().flatten().unwrap_or(0) as f64 / 1e6;
        match (interface, method) {
            (ROOT, "Raise") => (self.hooks.raise)(),
            (ROOT, "Quit") => (self.hooks.quit)(),
            (PLAYER, "Next") => (self.hooks.next)(),
            (PLAYER, "Previous") => (self.hooks.previous)(),
            (PLAYER, "Pause") => (self.hooks.pause)(),
            (PLAYER, "PlayPause") => (self.hooks.play_pause)(),
            (PLAYER, "Stop") => (self.hooks.stop)(),
            (PLAYER, "Play") => (self.hooks.play)(),
            (PLAYER, "Seek") => (self.hooks.seek_by)(seconds(0)),
            (PLAYER, "SetPosition") => (self.hooks.seek_to)(seconds(1)),
            (PLAYER, "OpenUri") => {}
            _ => {
                invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "No such method");
                return;
            }
        }
        invocation.return_value(None);
    }

    fn get(&self, interface: &str, property: &str) -> glib::Variant {
        let now = self.now.borrow();
        let none: Vec<String> = Vec::new();
        match (interface, property) {
            (ROOT, "CanQuit") | (ROOT, "CanRaise") => true.to_variant(),
            (ROOT, "HasTrackList") => false.to_variant(),
            (ROOT, "Identity") => "Matinee".to_variant(),
            (ROOT, "DesktopEntry") => crate::APP_ID.to_variant(),
            (ROOT, "SupportedUriSchemes") | (ROOT, "SupportedMimeTypes") => none.to_variant(),
            (PLAYER, "PlaybackStatus") => now.status.name().to_variant(),
            (PLAYER, "LoopStatus") => "None".to_variant(),
            (PLAYER, "Rate") => now.rate.max(0.01).to_variant(),
            (PLAYER, "Shuffle") => false.to_variant(),
            (PLAYER, "Metadata") => metadata(&now),
            (PLAYER, "Volume") => now.volume.to_variant(),
            (PLAYER, "Position") => (((self.hooks.position)() * 1e6) as i64).to_variant(),
            (PLAYER, "MinimumRate") => 1.0f64.to_variant(),
            (PLAYER, "MaximumRate") => 2.0f64.to_variant(),
            (PLAYER, "CanGoNext") => now.can_go_next.to_variant(),
            (PLAYER, "CanGoPrevious") => now.can_go_previous.to_variant(),
            (PLAYER, "CanPlay") | (PLAYER, "CanPause") | (PLAYER, "CanSeek") => (now.status != Status::Stopped).to_variant(),
            (PLAYER, "CanControl") => true.to_variant(),
            _ => false.to_variant(),
        }
    }

    fn set(&self, interface: &str, property: &str, value: &glib::Variant) -> bool {
        match (interface, property) {
            (PLAYER, "Volume") => {
                if let Some(volume) = value.get::<f64>() {
                    (self.hooks.set_volume)(volume.clamp(0.0, 1.0));
                }
                true
            }
            // Accepted and ignored: Matinee has no loop or shuffle, and the
            // speed is set from the player.
            (PLAYER, "LoopStatus") | (PLAYER, "Shuffle") | (PLAYER, "Rate") => true,
            _ => false,
        }
    }

    /// Tells the controls what's playing now; only what changed is sent.
    pub fn update(&self, now: Now) {
        let previous = self.now.replace(now.clone());
        if previous == now {
            return;
        }
        let changed = glib::VariantDict::new(None);
        if previous.status != now.status {
            changed.insert_value("PlaybackStatus", &now.status.name().to_variant());
            let can = (now.status != Status::Stopped).to_variant();
            for name in ["CanPlay", "CanPause", "CanSeek"] {
                changed.insert_value(name, &can);
            }
        }
        let same_track = previous.id == now.id
            && previous.title == now.title
            && previous.channel == now.channel
            && previous.art == now.art
            && previous.length == now.length;
        if !same_track {
            changed.insert_value("Metadata", &metadata(&now));
        }
        if previous.volume != now.volume {
            changed.insert_value("Volume", &now.volume.to_variant());
        }
        if previous.rate != now.rate {
            changed.insert_value("Rate", &now.rate.max(0.01).to_variant());
        }
        if previous.can_go_next != now.can_go_next {
            changed.insert_value("CanGoNext", &now.can_go_next.to_variant());
        }
        if previous.can_go_previous != now.can_go_previous {
            changed.insert_value("CanGoPrevious", &now.can_go_previous.to_variant());
        }
        let invalidated: Vec<String> = Vec::new();
        let parameters = glib::Variant::tuple_from_iter([PLAYER.to_variant(), changed.end(), invalidated.to_variant()]);
        self.emit("org.freedesktop.DBus.Properties", "PropertiesChanged", &parameters);
    }

    /// The position jumped (a seek), to `seconds`.
    pub fn seeked(&self, seconds: f64) {
        if self.now.borrow().status == Status::Stopped {
            return;
        }
        let parameters = glib::Variant::tuple_from_iter([((seconds * 1e6) as i64).to_variant()]);
        self.emit(PLAYER, "Seeked", &parameters);
    }

    fn emit(&self, interface: &str, signal: &str, parameters: &glib::Variant) {
        if let Some(connection) = self.connection.borrow().as_ref() {
            let _ = connection.emit_signal(None, PATH, interface, signal, Some(parameters));
        }
    }
}

impl Drop for Mpris {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.borrow_mut().take() {
            for id in self.registrations.borrow_mut().drain(..) {
                let _ = connection.unregister_object(id);
            }
        }
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
    }
}

/// The track's details in MPRIS's words.
fn metadata(now: &Now) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    let track = if now.id.is_empty() {
        NO_TRACK.to_string()
    } else {
        // Object paths allow letters, digits and underscores only.
        let safe: String = now.id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        format!("/io/github/matinee/track/{safe}")
    };
    if let Ok(path) = glib::variant::ObjectPath::try_from(track) {
        dict.insert_value("mpris:trackid", &path.to_variant());
    }
    if now.id.is_empty() {
        return dict.end();
    }
    if now.length > 0.0 {
        dict.insert_value("mpris:length", &((now.length * 1e6) as i64).to_variant());
    }
    if let Some(art) = now.art.as_ref().filter(|art| art.is_file()) {
        dict.insert_value("mpris:artUrl", &format!("file://{}", art.display()).to_variant());
    }
    dict.insert_value("xesam:title", &now.title.to_variant());
    dict.insert_value("xesam:artist", &vec![now.channel.clone()].to_variant());
    dict.insert_value("xesam:url", &now.url.to_variant());
    dict.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_names_the_track_safely() {
        let now = Now {
            id: "a-b_c9".into(),
            title: "Lathe".into(),
            channel: "Tom".into(),
            url: "https://www.youtube.com/watch?v=a-b_c9".into(),
            length: 90.5,
            ..Now::default()
        };
        let dict = glib::VariantDict::new(Some(&metadata(&now)));
        let track: glib::variant::ObjectPath = dict.lookup("mpris:trackid").unwrap().unwrap();
        assert_eq!(track.as_str(), "/io/github/matinee/track/a_b_c9");
        assert_eq!(dict.lookup::<i64>("mpris:length").unwrap(), Some(90_500_000));
        assert_eq!(dict.lookup::<Vec<String>>("xesam:artist").unwrap(), Some(vec!["Tom".to_string()]));
        assert!(dict.lookup::<String>("mpris:artUrl").unwrap().is_none());
    }

    #[test]
    fn nothing_playing_has_no_track() {
        let dict = glib::VariantDict::new(Some(&metadata(&Now::default())));
        let track: glib::variant::ObjectPath = dict.lookup("mpris:trackid").unwrap().unwrap();
        assert_eq!(track.as_str(), NO_TRACK);
        assert!(dict.lookup::<String>("xesam:title").unwrap().is_none());
    }
}
