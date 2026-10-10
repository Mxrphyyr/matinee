//! The thumbnail picture. It draws itself so that every effect on it (the
//! fade-up when an image arrives, the hover zoom, the dimming of watched
//! videos and the resume bar) is a plain repaint: no relayout, no offscreen
//! filter passes, and the frame keeps its 16:9 shape before the image exists.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use std::cell::{Cell, RefCell};

/// How far a thumbnail is zoomed while it fades up, and on hover.
const ARRIVAL_ZOOM: f64 = 0.06;
pub const HOVER_ZOOM: f64 = 1.045;

mod imp {
    use super::*;

    pub struct ThumbPaintable {
        pub texture: RefCell<Option<gdk::Texture>>,
        /// 0 = not yet visible, 1 = fully arrived.
        pub reveal: Cell<f64>,
        /// 1 = resting, HOVER_ZOOM = hovered.
        pub hover: Cell<f64>,
        /// How much to darken a watched video, 0–1.
        pub dim: Cell<f64>,
        /// Resume position as a fraction of the video; 0 hides the bar.
        pub progress: Cell<f64>,
    }

    impl Default for ThumbPaintable {
        fn default() -> Self {
            Self {
                texture: RefCell::new(None),
                reveal: Cell::new(1.0),
                hover: Cell::new(1.0),
                dim: Cell::new(0.0),
                progress: Cell::new(0.0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThumbPaintable {
        const NAME: &'static str = "MatineeThumbPaintable";
        type Type = super::ThumbPaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for ThumbPaintable {}

    impl PaintableImpl for ThumbPaintable {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::SIZE
        }

        fn intrinsic_width(&self) -> i32 {
            16
        }

        fn intrinsic_height(&self) -> i32 {
            9
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            16.0 / 9.0
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() else { return };
            let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            snapshot.push_clip(&bounds);

            if let Some(texture) = self.texture.borrow().as_ref() {
                let reveal = self.reveal.get().clamp(0.0, 1.0);
                let hover = self.hover.get();
                let (tw, th) = (f64::from(texture.width()), f64::from(texture.height()));
                // YouTube's hqdefault is 4:3 with black bars around a 16:9 picture;
                // fit that inner picture to the frame so the bars fall outside it.
                let content_height = if (tw / th - 4.0 / 3.0).abs() < 0.02 { tw * 9.0 / 16.0 } else { th };
                let zoom = hover * (1.0 + ARRIVAL_ZOOM * (1.0 - reveal));
                let scale = (width / tw).max(height / content_height) * zoom;
                let (draw_w, draw_h) = (tw * scale, th * scale);
                let target = graphene::Rect::new(
                    ((width - draw_w) / 2.0) as f32,
                    ((height - draw_h) / 2.0) as f32,
                    draw_w as f32,
                    draw_h as f32,
                );
                if reveal < 1.0 {
                    snapshot.push_opacity(reveal);
                }
                snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Trilinear, &target);
                // Watched videos sit back; hovering brings them most of the way forward.
                let hovered = ((hover - 1.0) / (HOVER_ZOOM - 1.0)).clamp(0.0, 1.0);
                let dim = self.dim.get() * (1.0 - 0.6 * hovered);
                if dim > 0.0 {
                    snapshot.append_color(&gdk::RGBA::new(0.07, 0.05, 0.09, dim as f32), &bounds);
                }
                if reveal < 1.0 {
                    snapshot.pop();
                }
            }

            let progress = self.progress.get().clamp(0.0, 1.0);
            if progress > 0.0 {
                let bar = 4.0_f32;
                let y = height as f32 - bar;
                snapshot.append_color(
                    &gdk::RGBA::new(1.0, 1.0, 1.0, 0.28),
                    &graphene::Rect::new(0.0, y, width as f32, bar),
                );
                snapshot.append_color(
                    &gdk::RGBA::new(0.949, 0.710, 0.267, 1.0),
                    &graphene::Rect::new(0.0, y, (width * progress) as f32, bar),
                );
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct ThumbPaintable(ObjectSubclass<imp::ThumbPaintable>)
        @implements gdk::Paintable;
}

impl Default for ThumbPaintable {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ThumbPaintable {
    pub fn set_texture(&self, texture: Option<&gdk::Texture>) {
        *self.imp().texture.borrow_mut() = texture.cloned();
        self.invalidate_contents();
    }

    pub fn has_texture(&self) -> bool {
        self.imp().texture.borrow().is_some()
    }

    pub fn set_reveal(&self, value: f64) {
        self.imp().reveal.set(value);
        self.invalidate_contents();
    }

    pub fn hover(&self) -> f64 {
        self.imp().hover.get()
    }

    pub fn set_hover(&self, value: f64) {
        self.imp().hover.set(value);
        self.invalidate_contents();
    }

    pub fn set_dim(&self, value: f64) {
        self.imp().dim.set(value);
        self.invalidate_contents();
    }

    pub fn set_progress(&self, fraction: f64) {
        self.imp().progress.set(fraction);
        self.invalidate_contents();
    }
}
