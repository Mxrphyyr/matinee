// Finds libmpv (mpv's playback engine), which draws video inside the window.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if let Err(error) = pkg_config::Config::new().atleast_version("2.0").probe("mpv") {
        panic!(
            "\n\nMatinee needs libmpv's development files (mpv.pc, version 2.0 / mpv 0.35 or newer).\n\
             On AerynOS: sudo moss install 'pkgconfig(mpv)'\n\n{error}\n"
        );
    }
}
