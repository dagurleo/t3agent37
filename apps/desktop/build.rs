// The menu-bar app's AppKit side is Objective-C (src/bin/t3agent37/macos/shell.m). The command
// alone (`--no-default-features`, no panel) has none.
fn main() {
    println!("cargo:rerun-if-changed=src/bin/t3agent37/macos/shell.m");
    let panel = std::env::var_os("CARGO_FEATURE_PANEL").is_some();
    if !panel || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new().file("src/bin/t3agent37/macos/shell.m").flag("-fobjc-arc").flag("-fmodules").compile("t3a_shell");
    for framework in ["AppKit", "QuartzCore", "ServiceManagement", "UserNotifications"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
