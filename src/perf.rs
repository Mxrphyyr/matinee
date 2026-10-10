//! Timing diagnostics, printed when Matinee runs with MATINEE_PERF=1: what
//! start-up spends its time on, how many frames arrive late and the longest
//! bits of work behind them, and what the player is doing. The owner runs
//! `MATINEE_PERF=1 ~/.local/bin/matinee` and sends the output; with the
//! variable unset none of this costs anything.

use gtk::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MATINEE_PERF").is_some_and(|v| v == "1"))
}

fn launch() -> Instant {
    static LAUNCH: OnceLock<Instant> = OnceLock::new();
    *LAUNCH.get_or_init(Instant::now)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Starts the clock; the first thing `main` does.
pub fn init() {
    let _ = launch();
    if enabled() {
        eprintln!("[perf] Matinee {}; times are milliseconds since launch", env!("CARGO_PKG_VERSION"));
    }
}

/// Notes a moment, such as "window shown".
pub fn mark(what: &str) {
    if enabled() {
        eprintln!("[perf] {:8.1}  {what}", ms(launch().elapsed()));
    }
}

/// A stretch of work on the UI thread, timed until dropped. Stretches of
/// `warn_ms` or longer are printed; the longest go into the frame report.
pub struct Span {
    what: String,
    started: Instant,
    warn: Duration,
}

pub fn span(what: impl Into<String>, warn_ms: u64) -> Option<Span> {
    enabled().then(|| Span { what: what.into(), started: Instant::now(), warn: Duration::from_millis(warn_ms) })
}

impl Drop for Span {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        if took >= self.warn {
            eprintln!("[perf] {:8.1}  {} took {:.2} ms", ms(launch().elapsed()), self.what, ms(took));
        }
        REPORT.with(|report| report.borrow_mut().note(&self.what, took));
    }
}

/// Frames and long stretches since the last report.
#[derive(Default)]
struct Report {
    refresh: f64,
    last_tick: i64,
    frames: u32,
    late: u32,
    very_late: u32,
    worst: f64,
    longest: Vec<(f64, String)>,
}

thread_local! {
    static REPORT: RefCell<Report> = RefCell::default();
}

impl Report {
    fn note(&mut self, what: &str, took: Duration) {
        self.longest.push((ms(took), what.to_string()));
        self.longest.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.longest.truncate(4);
    }

    fn tick(&mut self, now: i64, refresh: f64) {
        self.refresh = refresh;
        let since = (now - self.last_tick) as f64 / 1000.0;
        self.last_tick = now;
        // The first tick, and a gap over a second (the window idle or
        // hidden), aren't frames.
        if since <= 0.0 || since > 1000.0 {
            return;
        }
        self.frames += 1;
        if since > refresh * 1.5 {
            self.late += 1;
        }
        if since > refresh * 3.0 {
            self.very_late += 1;
        }
        self.worst = self.worst.max(since);
    }

    fn print(&mut self) {
        if self.frames == 0 {
            return;
        }
        eprintln!(
            "[perf] {:8.1}  frames: {} in 5 s at {:.1} ms refresh; late (>{:.0} ms): {}, very late (>{:.0} ms): {}, worst {:.1} ms",
            ms(launch().elapsed()),
            self.frames,
            self.refresh,
            self.refresh * 1.5,
            self.late,
            self.refresh * 3.0,
            self.very_late,
            self.worst
        );
        if !self.longest.is_empty() {
            let list: Vec<String> = self.longest.iter().map(|(t, what)| format!("{what} {t:.1} ms")).collect();
            eprintln!("[perf]           longest work: {}", list.join(", "));
        }
        *self = Report { refresh: self.refresh, last_tick: self.last_tick, ..Report::default() };
    }
}

/// Watches every frame of `window` and prints a report every five seconds.
/// The frame clock is kept running so quiet moments count as frames too.
pub fn watch_frames(window: &impl IsA<gtk::Widget>) {
    if !enabled() {
        return;
    }
    let first = std::cell::Cell::new(true);
    window.add_tick_callback(move |widget, clock| {
        if first.replace(false) {
            clock.begin_updating();
            let renderer = widget
                .native()
                .and_then(|native| native.renderer())
                .map_or("unknown".to_string(), |renderer| renderer.type_().name().to_string());
            mark(&format!("first frame; GTK renderer {renderer}"));
        }
        let (refresh, _) = clock.refresh_info(clock.frame_time());
        let refresh = if refresh > 0 { refresh as f64 / 1000.0 } else { 16.7 };
        REPORT.with(|report| report.borrow_mut().tick(clock.frame_time(), refresh));
        glib::ControlFlow::Continue
    });
    glib::timeout_add_seconds_local(5, || {
        REPORT.with(|report| report.borrow_mut().print());
        glib::ControlFlow::Continue
    });
}
