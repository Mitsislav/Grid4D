//! Windows: embeds the application icon and version information in the
//! executable, so Explorer, shortcuts and the taskbar show Grid4D's icon (the
//! running window sets its own in `main.rs`). Other targets need nothing here.

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=assets/icon/grid4d.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon/grid4d.ico")
            .set("ProductName", "Grid4D")
            .set("FileDescription", "Grid4D")
            .compile()?;
    }
    Ok(())
}
