//! Thumbnails load on six worker threads, are decoded off the UI thread, and
//! are kept on disk in ~/.cache/matinee/thumbs so the next launch is instant.
//!
//! The queue is newest-first: when you scroll, what you're looking at now
//! loads before what you scrolled past, and requests for cards that left the
//! screen are dropped. Finished images are handed to the UI a few at a time
//! so a burst of arrivals never lands in a single frame.

use crate::model::is_video_id;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

type Waiter = Box<dyn FnOnce(&gdk::Texture)>;

const WORKERS: usize = 6;
/// Recently shown images kept decoded; older ones reload from disk in milliseconds.
const MEMORY_CACHE: usize = 120;
const DISK_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

#[derive(Default)]
struct Queue {
    jobs: Mutex<VecDeque<String>>,
    ready: Condvar,
}

pub struct Thumbs {
    queue: Arc<Queue>,
    cache: RefCell<HashMap<String, gdk::Texture>>,
    order: RefCell<VecDeque<String>>,
    waiting: RefCell<HashMap<String, Vec<(u64, Waiter)>>>,
    next_token: Cell<u64>,
}

impl Thumbs {
    pub fn new() -> Rc<Self> {
        let dir = glib::user_cache_dir().join("matinee").join("thumbs");
        let _ = std::fs::create_dir_all(&dir);
        let queue = Arc::new(Queue::default());
        let (done_tx, done_rx) = async_channel::unbounded::<(String, Option<gdk::Texture>)>();

        for _ in 0..WORKERS {
            let queue = queue.clone();
            let done_tx = done_tx.clone();
            let dir = dir.clone();
            std::thread::spawn(move || loop {
                let id = {
                    let mut jobs = queue.jobs.lock().unwrap();
                    loop {
                        // Newest first: the cards on screen right now.
                        if let Some(id) = jobs.pop_back() {
                            break id;
                        }
                        jobs = queue.ready.wait(jobs).unwrap();
                    }
                };
                let texture = load(&dir, &id);
                if done_tx.send_blocking((id, texture)).is_err() {
                    break;
                }
            });
        }
        std::thread::spawn(move || prune(&dir));

        let this = Rc::new(Self {
            queue,
            cache: RefCell::default(),
            order: RefCell::default(),
            waiting: RefCell::default(),
            next_token: Cell::new(1),
        });
        let weak = Rc::downgrade(&this);
        glib::spawn_future_local(async move {
            while let Ok((id, texture)) = done_rx.recv().await {
                let Some(this) = weak.upgrade() else { break };
                this.finish(id, texture);
                drop(this);
                // Spread arrivals over frames instead of uploading a burst at once.
                if !done_rx.is_empty() {
                    glib::timeout_future(Duration::from_millis(4)).await;
                }
            }
        });
        this
    }

    /// Calls `done` with the video's thumbnail: right away if it's in memory,
    /// otherwise once loaded. Returns a token for `cancel` when it has to wait.
    pub fn get(&self, video_id: &str, done: impl FnOnce(&gdk::Texture) + 'static) -> Option<u64> {
        if !is_video_id(video_id) {
            return None;
        }
        let cached = self.cache.borrow().get(video_id).cloned();
        if let Some(texture) = cached {
            done(&texture);
            return None;
        }
        let token = self.next_token.get();
        self.next_token.set(token + 1);
        let mut waiting = self.waiting.borrow_mut();
        let waiters = waiting.entry(video_id.to_string()).or_default();
        waiters.push((token, Box::new(done)));
        if waiters.len() == 1 {
            self.queue.jobs.lock().unwrap().push_back(video_id.to_string());
            self.queue.ready.notify_one();
        }
        Some(token)
    }

    /// Forget a request whose card scrolled away. If nobody else wants that
    /// image it leaves the queue, unless a worker already started on it.
    pub fn cancel(&self, video_id: &str, token: u64) {
        let mut waiting = self.waiting.borrow_mut();
        let Some(waiters) = waiting.get_mut(video_id) else { return };
        waiters.retain(|(t, _)| *t != token);
        if waiters.is_empty() {
            waiting.remove(video_id);
            self.queue.jobs.lock().unwrap().retain(|queued| queued != video_id);
        }
    }

    fn finish(&self, id: String, texture: Option<gdk::Texture>) {
        let waiters = self.waiting.borrow_mut().remove(&id).unwrap_or_default();
        let Some(texture) = texture else { return };
        {
            let mut cache = self.cache.borrow_mut();
            let mut order = self.order.borrow_mut();
            if cache.insert(id.clone(), texture.clone()).is_none() {
                order.push_back(id);
            }
            while order.len() > MEMORY_CACHE {
                if let Some(oldest) = order.pop_front() {
                    cache.remove(&oldest);
                }
            }
        }
        for (_, waiter) in waiters {
            waiter(&texture);
        }
    }
}

fn load(dir: &Path, id: &str) -> Option<gdk::Texture> {
    let path = dir.join(format!("{id}.jpg"));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if !bytes.is_empty() => bytes,
        _ => {
            // hqdefault is 4:3 with letterbox bars; the card crops it to 16:9,
            // which leaves a sharp 480px-wide image with the bars cut off.
            let url = format!("{}/vi/{id}/hqdefault.jpg", crate::net::thumb_base());
            let bytes = crate::net::get_bytes(&url).ok()?;
            let _ = std::fs::write(&path, &bytes);
            bytes
        }
    };
    // Texture decoding is thread-safe in GTK, so it stays off the UI thread.
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > DISK_MAX_AGE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
