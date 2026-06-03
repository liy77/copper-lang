// Windows resources for the installer .exe:
//   * the application ICON (shown in Explorer / the taskbar / Alt-Tab), and
//   * an `asInvoker` manifest, so Windows does NOT auto-elevate this binary.
//     Without it, the installer-detection heuristic flags any exe whose name
//     contains "installer"/"setup"/"update" and demands a UAC prompt — which
//     also made the process unkillable from a normal (non-elevated) shell.
// Other platforms: no-op.
fn main() {
    #[cfg(windows)]
    {
        const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#;
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/installer.ico");
        res.set_manifest(MANIFEST);
        if let Err(e) = res.compile() {
            // Best effort: a missing resource compiler shouldn't break the build,
            // but warn loudly so the icon/manifest aren't silently dropped.
            println!("cargo:warning=winresource failed (exe icon/manifest not embedded): {e}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/installer.ico");
}
