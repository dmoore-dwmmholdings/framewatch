fn main() {
    // ScreenCaptureKit's Swift bridge links against the system Swift runtime.
    // Dependency build-script linker arguments do not propagate to this final
    // binary, so add the system runtime rpath at the package boundary.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
