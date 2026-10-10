//! Smooth mouse-wheel scrolling. GTK jumps a fixed distance per wheel notch;
//! here each notch moves a target and the view glides toward it on every
//! frame, so quick notches add up into one continuous motion. Touchpads and
//! touchscreens keep GTK's own pixel-precise, kinetic scrolling.

use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Seconds for the remaining distance to shrink by about two thirds.
/// Small enough to feel immediate, large enough to read as motion.
const TIME_CONSTANT: f64 = 0.075;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
}

/// Smooth scrolling can be turned off in Preferences; GTK's steps return.
pub fn set_enabled(on: bool) {
    ENABLED.with(|enabled| enabled.set(on));
}

#[derive(Default)]
struct Glide {
    target: Cell<f64>,
    last_value: Cell<f64>,
    last_frame: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

pub fn attach(scroller: &gtk::ScrolledWindow) {
    let glide = Rc::new(Glide::default());
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    // Capture phase: decide before the scrolled window applies its own jump.
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = scroller.downgrade();
    wheel.connect_scroll(move |controller, _, dy| {
        let Some(scroller) = weak.upgrade() else { return glib::Propagation::Proceed };
        let modified = controller
            .current_event_state()
            .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK);
        if !ENABLED.with(Cell::get)
            || controller.unit() != gdk::ScrollUnit::Wheel
            || dy == 0.0
            || modified
            || !adw::is_animations_enabled(&scroller)
        {
            return glib::Propagation::Proceed;
        }
        let adjustment = scroller.vadjustment();
        let gliding = glide.tick.borrow().is_some();
        let from = if gliding { glide.target.get() } else { adjustment.value() };
        // GTK's own step for this view height, a touch longer since it now glides.
        let step = adjustment.page_size().powf(2.0 / 3.0) * 1.2;
        glide.target.set(clamp(&adjustment, from + dy * step));
        if !gliding {
            start(&scroller, &glide);
        }
        glib::Propagation::Stop
    });
    scroller.add_controller(wheel);
}

fn clamp(adjustment: &gtk::Adjustment, value: f64) -> f64 {
    let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    value.clamp(adjustment.lower(), max)
}

fn start(scroller: &gtk::ScrolledWindow, glide: &Rc<Glide>) {
    glide.last_frame.set(0);
    glide.last_value.set(scroller.vadjustment().value());
    let glide_ref = glide.clone();
    let id = scroller.add_tick_callback(move |scroller, clock| {
        let glide = &glide_ref;
        let adjustment = scroller.vadjustment();
        let moved = adjustment.value() - glide.last_value.get();
        if moved.abs() > adjustment.page_size() * 0.5 {
            // A big jump is someone else's doing (scrollbar, keys): let it win.
            glide.tick.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        if moved.abs() > 0.5 {
            // The list nudged itself to keep its place as rows were measured;
            // carry the goal along instead of snapping back.
            glide.target.set(glide.target.get() + moved);
        }
        let now = clock.frame_time();
        let dt = match glide.last_frame.get() {
            0 => 1.0 / 60.0,
            last => ((now - last) as f64 / 1_000_000.0).clamp(0.001, 0.1),
        };
        glide.last_frame.set(now);
        let target = clamp(&adjustment, glide.target.get());
        let remaining = target - adjustment.value();
        if remaining.abs() < 0.5 {
            adjustment.set_value(target);
            glide.tick.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        // Exponential approach: frame-rate independent, never overshoots.
        adjustment.set_value(adjustment.value() + remaining * (1.0 - (-dt / TIME_CONSTANT).exp()));
        glide.last_value.set(adjustment.value());
        glib::ControlFlow::Continue
    });
    *glide.tick.borrow_mut() = Some(id);
}
