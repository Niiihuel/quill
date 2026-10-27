fn main() {
    // shortcut: a native Windows build only (the build-dependency is the host's), cross-compiling
    // from Linux leaves the exe without its icon; check CARGO_CFG_TARGET_OS if that is needed.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/icons/quill.ico")
        .compile()
        .expect("quill's icon goes into the Windows executable");
}
