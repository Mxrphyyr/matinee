//! A small, safe binding to libmpv covering exactly what the embedded player
//! needs. libmpv is mpv as a library, so formats, hardware decoding and
//! YouTube streaming (through yt-dlp) behave exactly as in the mpv program.
//! Struct layouts and constants follow mpv's client.h, render.h and
//! render_gl.h for client API 2.x (mpv 0.35 and newer).

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::rc::Rc;

pub use gl::current_framebuffer;

#[repr(C)]
struct Handle {
    _opaque: [u8; 0],
}

#[repr(C)]
struct RenderHandle {
    _opaque: [u8; 0],
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

type GetProcAddress = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

#[repr(C)]
struct GlInitParams {
    get_proc_address: Option<GetProcAddress>,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct GlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

/// Only the leading fields of mpv_event_end_file, which are all that's read.
#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct RawLogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}

const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_NONE: c_int = 0;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_LOG_MESSAGE: c_int = 2;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;
const END_FILE_REASON_ERROR: c_int = 4;

const RENDER_PARAM_INVALID: c_int = 0;
const RENDER_PARAM_API_TYPE: c_int = 1;
const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const RENDER_PARAM_OPENGL_FBO: c_int = 3;
const RENDER_PARAM_FLIP_Y: c_int = 4;
const RENDER_PARAM_X11_DISPLAY: c_int = 8;
const RENDER_PARAM_WL_DISPLAY: c_int = 9;
const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;

/// LC_NUMERIC on glibc and musl.
const LC_NUMERIC: c_int = 1;

extern "C" {
    fn mpv_create() -> *mut Handle;
    fn mpv_initialize(ctx: *mut Handle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut Handle);
    fn mpv_set_option_string(ctx: *mut Handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_command(ctx: *mut Handle, args: *mut *const c_char) -> c_int;
    fn mpv_command_async(ctx: *mut Handle, reply_userdata: u64, args: *mut *const c_char) -> c_int;
    fn mpv_set_property(ctx: *mut Handle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_set_property_string(ctx: *mut Handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_get_property(ctx: *mut Handle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_observe_property(ctx: *mut Handle, reply_userdata: u64, name: *const c_char, format: c_int) -> c_int;
    fn mpv_request_log_messages(ctx: *mut Handle, min_level: *const c_char) -> c_int;
    fn mpv_wait_event(ctx: *mut Handle, timeout: f64) -> *mut RawEvent;
    fn mpv_set_wakeup_callback(ctx: *mut Handle, cb: Option<unsafe extern "C" fn(*mut c_void)>, data: *mut c_void);
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_render_context_create(res: *mut *mut RenderHandle, mpv: *mut Handle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_set_update_callback(
        ctx: *mut RenderHandle,
        cb: Option<unsafe extern "C" fn(*mut c_void)>,
        data: *mut c_void,
    );
    fn mpv_render_context_render(ctx: *mut RenderHandle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_free(ctx: *mut RenderHandle);
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

/// Called from mpv's own threads, so it may only do thread-safe work.
type Callback = Box<dyn Fn() + Send + Sync>;

unsafe extern "C" fn call_back(data: *mut c_void) {
    let callback = &*(data as *const Callback);
    callback();
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn check(code: c_int) -> Result<(), String> {
    if code >= 0 {
        Ok(())
    } else {
        Err(error_string(code))
    }
}

pub fn error_string(code: i32) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)) }
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Copy)]
pub enum Value {
    Flag(bool),
    Double(f64),
    Unavailable,
}

pub enum Format {
    Flag,
    Double,
}

pub enum Event {
    FileLoaded,
    /// Playback (re)started after loading or seeking: a frame is ready.
    PlaybackRestart,
    EndFile { failed: bool, code: i32 },
    Property { name: String, value: Value },
    Log { prefix: String, text: String },
    Shutdown,
}

pub struct Mpv {
    handle: *mut Handle,
    wakeup: Box<Callback>,
}

impl Mpv {
    /// Creates and starts an mpv instance. `wakeup` runs on an mpv thread
    /// whenever events are waiting; drain them with `next_event`.
    pub fn new(options: &[(&str, &str)], wakeup: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        // libmpv refuses to start unless numbers are formatted the C way.
        unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
        let handle = unsafe { mpv_create() };
        if handle.is_null() {
            return Err("libmpv couldn't be created".into());
        }
        let wakeup: Box<Callback> = Box::new(Box::new(wakeup));
        let mpv = Mpv { handle, wakeup };
        for (name, value) in options {
            if let Err(e) = mpv.set_option(name, value) {
                if *name == "vo" {
                    return Err(format!("this libmpv can't draw inside apps ({e})"));
                }
                // Builds differ (Lua, hardware decoding); a missing extra isn't fatal.
                eprintln!("matinee: mpv ignored option {name}={value}: {e}");
            }
        }
        check(unsafe { mpv_initialize(mpv.handle) })?;
        let data = &*mpv.wakeup as *const Callback as *mut c_void;
        unsafe { mpv_set_wakeup_callback(mpv.handle, Some(call_back), data) };
        Ok(mpv)
    }

    fn set_option(&self, name: &str, value: &str) -> Result<(), String> {
        let (name, value) = (cstring(name), cstring(value));
        check(unsafe { mpv_set_option_string(self.handle, name.as_ptr(), value.as_ptr()) })
    }

    fn args(args: &[&str]) -> (Vec<CString>, Vec<*const c_char>) {
        let owned: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        pointers.push(ptr::null());
        (owned, pointers)
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let (_owned, mut pointers) = Self::args(args);
        check(unsafe { mpv_command(self.handle, pointers.as_mut_ptr()) })
    }

    /// Runs a command without waiting for it to finish.
    pub fn command_async(&self, args: &[&str]) {
        let (_owned, mut pointers) = Self::args(args);
        unsafe { mpv_command_async(self.handle, 0, pointers.as_mut_ptr()) };
    }

    pub fn set_flag(&self, name: &str, value: bool) {
        let name = cstring(name);
        let mut data = c_int::from(value);
        unsafe { mpv_set_property(self.handle, name.as_ptr(), FORMAT_FLAG, &mut data as *mut c_int as *mut c_void) };
    }

    pub fn set_double(&self, name: &str, value: f64) {
        let name = cstring(name);
        let mut data = value;
        unsafe { mpv_set_property(self.handle, name.as_ptr(), FORMAT_DOUBLE, &mut data as *mut f64 as *mut c_void) };
    }

    pub fn set_string(&self, name: &str, value: &str) {
        let (name, value) = (cstring(name), cstring(value));
        unsafe { mpv_set_property_string(self.handle, name.as_ptr(), value.as_ptr()) };
    }

    pub fn double(&self, name: &str) -> Option<f64> {
        let name = cstring(name);
        let mut data: f64 = 0.0;
        let code = unsafe { mpv_get_property(self.handle, name.as_ptr(), FORMAT_DOUBLE, &mut data as *mut f64 as *mut c_void) };
        (code >= 0).then_some(data)
    }

    pub fn observe(&self, name: &str, format: Format) {
        let name = cstring(name);
        let format = match format {
            Format::Flag => FORMAT_FLAG,
            Format::Double => FORMAT_DOUBLE,
        };
        unsafe { mpv_observe_property(self.handle, 0, name.as_ptr(), format) };
    }

    pub fn request_log_messages(&self, level: &str) {
        let level = cstring(level);
        unsafe { mpv_request_log_messages(self.handle, level.as_ptr()) };
    }

    /// The next waiting event, or None when there are no more right now.
    pub fn next_event(&self) -> Option<Event> {
        let text = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        loop {
            // mpv owns the event; it stays valid until the next call.
            let raw = unsafe { &*mpv_wait_event(self.handle, 0.0) };
            let event = match raw.event_id {
                EVENT_NONE => return None,
                EVENT_SHUTDOWN => Event::Shutdown,
                EVENT_FILE_LOADED => Event::FileLoaded,
                EVENT_PLAYBACK_RESTART => Event::PlaybackRestart,
                EVENT_END_FILE if !raw.data.is_null() => {
                    let end = unsafe { &*(raw.data as *const RawEndFile) };
                    Event::EndFile { failed: end.reason == END_FILE_REASON_ERROR, code: end.error }
                }
                EVENT_PROPERTY_CHANGE if !raw.data.is_null() => {
                    let property = unsafe { &*(raw.data as *const RawProperty) };
                    let value = match property.format {
                        FORMAT_FLAG if !property.data.is_null() => {
                            Value::Flag(unsafe { *(property.data as *const c_int) } != 0)
                        }
                        FORMAT_DOUBLE if !property.data.is_null() => {
                            Value::Double(unsafe { *(property.data as *const f64) })
                        }
                        _ => Value::Unavailable,
                    };
                    Event::Property { name: text(property.name), value }
                }
                EVENT_LOG_MESSAGE if !raw.data.is_null() => {
                    let log = unsafe { &*(raw.data as *const RawLogMessage) };
                    Event::Log { prefix: text(log.prefix), text: text(log.text) }
                }
                _ => continue,
            };
            return Some(event);
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe {
            mpv_set_wakeup_callback(self.handle, None, ptr::null_mut());
            mpv_terminate_destroy(self.handle);
        }
    }
}

/// The windowing-system connection. Handing it to mpv lets hardware-decoded
/// frames go straight to the GPU's renderer instead of being copied back
/// through system memory (VA-API and similar).
pub enum NativeDisplay {
    Wayland(*mut c_void),
    X11(*mut c_void),
    Unknown,
}

pub fn native_display(display: &gtk::gdk::Display) -> NativeDisplay {
    use gtk::glib::prelude::*;
    use gtk::glib::translate::ToGlibPtr;
    let raw: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
    type GetNative = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    // Looked up at run time: GTK may be built without one of the backends.
    let get = |name: &CStr| unsafe { gl::global_symbol::<GetNative>(name) };
    match display.type_().name() {
        "GdkWaylandDisplay" => get(c"gdk_wayland_display_get_wl_display")
            .map_or(NativeDisplay::Unknown, |f| NativeDisplay::Wayland(unsafe { f(raw.cast()) })),
        "GdkX11Display" => get(c"gdk_x11_display_get_xdisplay")
            .map_or(NativeDisplay::Unknown, |f| NativeDisplay::X11(unsafe { f(raw.cast()) })),
        _ => NativeDisplay::Unknown,
    }
}

/// Draws mpv's video into an OpenGL framebuffer. Create, draw and drop it
/// only while the target OpenGL context is current.
pub struct Render {
    handle: *mut RenderHandle,
    _update: Box<Callback>,
    // Keeps mpv alive until the render context is gone, as libmpv requires.
    _mpv: Rc<Mpv>,
}

impl Render {
    /// `on_update` runs on an mpv thread whenever a new frame should be drawn.
    pub fn new(
        mpv: Rc<Mpv>,
        display: NativeDisplay,
        on_update: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let mut init = GlInitParams {
            get_proc_address: Some(gl::proc_address),
            get_proc_address_ctx: ptr::null_mut(),
        };
        let mut params = vec![
            RenderParam { kind: RENDER_PARAM_API_TYPE, data: c"opengl".as_ptr() as *mut c_void },
            RenderParam {
                kind: RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut GlInitParams as *mut c_void,
            },
        ];
        match display {
            NativeDisplay::Wayland(raw) if !raw.is_null() => {
                params.push(RenderParam { kind: RENDER_PARAM_WL_DISPLAY, data: raw })
            }
            NativeDisplay::X11(raw) if !raw.is_null() => {
                params.push(RenderParam { kind: RENDER_PARAM_X11_DISPLAY, data: raw })
            }
            _ => {}
        }
        params.push(RenderParam { kind: RENDER_PARAM_INVALID, data: ptr::null_mut() });
        let mut handle = ptr::null_mut();
        check(unsafe { mpv_render_context_create(&mut handle, mpv.handle, params.as_mut_ptr()) })?;
        let update: Box<Callback> = Box::new(Box::new(on_update));
        let data = &*update as *const Callback as *mut c_void;
        unsafe { mpv_render_context_set_update_callback(handle, Some(call_back), data) };
        Ok(Render { handle, _update: update, _mpv: mpv })
    }

    /// Draws the current frame into `framebuffer` (sizes in device pixels).
    /// Never waits for the frame's display time, so the UI never stalls.
    pub fn draw(&self, framebuffer: i32, width: i32, height: i32) {
        let mut target = GlFbo { fbo: framebuffer, w: width, h: height, internal_format: 0 };
        let mut flip: c_int = 1;
        let mut block: c_int = 0;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_OPENGL_FBO, data: &mut target as *mut GlFbo as *mut c_void },
            RenderParam { kind: RENDER_PARAM_FLIP_Y, data: &mut flip as *mut c_int as *mut c_void },
            RenderParam {
                kind: RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: &mut block as *mut c_int as *mut c_void,
            },
            RenderParam { kind: RENDER_PARAM_INVALID, data: ptr::null_mut() },
        ];
        unsafe { mpv_render_context_render(self.handle, params.as_mut_ptr()) };
    }
}

impl Drop for Render {
    fn drop(&mut self) {
        unsafe {
            mpv_render_context_set_update_callback(self.handle, None, ptr::null_mut());
            mpv_render_context_free(self.handle);
        }
    }
}

/// OpenGL function lookup for whichever API GTK picked: EGL (Wayland, and
/// X11 on recent GTK) or GLX (older X11 setups).
mod gl {
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::sync::OnceLock;

    extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;

    type GetCurrentContext = unsafe extern "C" fn() -> *mut c_void;
    type GetProc = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type GetIntegerv = unsafe extern "C" fn(u32, *mut c_int);

    struct Loader {
        egl_current_context: Option<GetCurrentContext>,
        egl_proc_address: Option<GetProc>,
        glx_proc_address: Option<GetProc>,
    }

    fn loader() -> &'static Loader {
        static LOADER: OnceLock<Loader> = OnceLock::new();
        LOADER.get_or_init(|| unsafe {
            let egl = open(&[c"libEGL.so.1", c"libEGL.so"]);
            let glx = open(&[c"libGLX.so.0", c"libGL.so.1", c"libGL.so"]);
            Loader {
                egl_current_context: symbol(egl, c"eglGetCurrentContext"),
                egl_proc_address: symbol(egl, c"eglGetProcAddress"),
                glx_proc_address: symbol(glx, c"glXGetProcAddressARB"),
            }
        })
    }

    unsafe fn open(names: &[&CStr]) -> *mut c_void {
        for name in names {
            let library = dlopen(name.as_ptr(), RTLD_NOW);
            if !library.is_null() {
                return library;
            }
        }
        std::ptr::null_mut()
    }

    /// A function already loaded into the process, such as one of GTK's.
    pub unsafe fn global_symbol<T: Copy>(name: &CStr) -> Option<T> {
        // RTLD_DEFAULT is a null handle on glibc and musl.
        let address = dlsym(std::ptr::null_mut(), name.as_ptr());
        if address.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&address))
        }
    }

    unsafe fn symbol<T: Copy>(library: *mut c_void, name: &CStr) -> Option<T> {
        if library.is_null() {
            return None;
        }
        let address = dlsym(library, name.as_ptr());
        if address.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&address))
        }
    }

    pub unsafe extern "C" fn proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        let loader = loader();
        if let (Some(current), Some(get)) = (loader.egl_current_context, loader.egl_proc_address) {
            if !current().is_null() {
                return get(name);
            }
        }
        match loader.glx_proc_address {
            Some(get) => get(name),
            None => std::ptr::null_mut(),
        }
    }

    /// The framebuffer GTK bound for the GLArea currently being drawn.
    pub fn current_framebuffer() -> i32 {
        const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
        static GET_INTEGERV: OnceLock<Option<GetIntegerv>> = OnceLock::new();
        let get = *GET_INTEGERV.get_or_init(|| {
            let address = unsafe { proc_address(std::ptr::null_mut(), c"glGetIntegerv".as_ptr()) };
            if address.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute::<*mut c_void, GetIntegerv>(address) })
            }
        });
        let mut framebuffer: c_int = 0;
        if let Some(get) = get {
            unsafe { get(GL_FRAMEBUFFER_BINDING, &mut framebuffer) };
        }
        framebuffer
    }
}
