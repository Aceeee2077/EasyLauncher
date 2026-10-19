fn main() {
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("Could not prepare EasyLauncher application resources");
    // The Rust test harness also links Tauri GUI code when exercising Manager.
    // It needs the same modern common-controls activation context as the app.
    if std::env::var("TARGET")
        .unwrap_or_default()
        .contains("windows-msvc")
    {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
}
