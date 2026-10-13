//! Refuses to build a release binary without the web UI.
//!
//! The binary embeds `webui/dist`. Debug builds may lack it (tests and development, where
//! Vite serves the UI), but a release binary without its UI would be broken.

use std::path::Path;

const WEB_UI_DIST: &str = "../../webui/dist";

fn main() {
    // Re-embed the UI whenever a new build of it appears.
    println!("cargo::rerun-if-changed={WEB_UI_DIST}");
    let index = Path::new(WEB_UI_DIST).join("index.html");
    if std::env::var("PROFILE").as_deref() == Ok("release") && !index.is_file() {
        println!(
            "cargo::error=webui/dist/index.html is missing. Build the web UI with `pnpm --dir webui build` before building a release."
        );
    }
}
