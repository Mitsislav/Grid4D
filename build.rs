//! The build's identity and, on Windows, the executable's resources.
//!
//! - `GRID4D_COMMIT` and `GRID4D_COMMIT_DATE`: the source commit (short hash)
//!   and its date (`YYYY-MM-DD`), shown in Help ▸ About; both empty when the
//!   source is not a git checkout. The commit's date, not the build's, so two
//!   builds of the same commit are identical.
//! - Windows: embeds the application icon and version information in the
//!   executable, so Explorer, shortcuts and the taskbar show Grid4D's icon (the
//!   running window sets its own in `main.rs`).

use std::process::Command;

/// The output of `git <args>`, trimmed; empty if git or the checkout is missing.
fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/icon/grid4d.ico");
    // A new commit (or a checkout of another) changes the identity.
    for path in [".git/HEAD", ".git/index"] {
        if std::path::Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!(
        "cargo:rustc-env=GRID4D_COMMIT={}",
        git(&["rev-parse", "--short=7", "HEAD"])
    );
    println!(
        "cargo:rustc-env=GRID4D_COMMIT_DATE={}",
        git(&["log", "-1", "--format=%cs"])
    );

    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon/grid4d.ico")
            .set("ProductName", "Grid4D")
            .set("FileDescription", "Grid4D")
            .compile()?;
    }
    Ok(())
}
