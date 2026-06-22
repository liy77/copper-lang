//! Build script for the Alloy GUI host.
//!
//! On Windows, embeds `assets/alloy-icon.ico` into the executable's PE header
//! so the `.exe` shows the Alloy icon in Explorer and the taskbar. (mocida-sys's
//! own build script handles linking + DLL staging; this only adds the icon.)
//!
//! On other platforms this is a no-op — macOS gets its icon from the `.app`
//! bundle produced by `packaging/make-app.sh`, not from the binary.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/alloy-icon.ico");
        let ico = std::path::Path::new("assets/alloy-icon.ico");
        if ico.is_file() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("assets/alloy-icon.ico");
            if let Err(e) = res.compile() {
                println!("cargo:warning=falha ao embutir o ícone do Alloy: {e}");
            }
        } else {
            println!(
                "cargo:warning=assets/alloy-icon.ico ausente — gere-o do PNG \
                 (assets/alloy-icon.png) para embutir o ícone no .exe"
            );
        }
    }
}
