fn main() {
    println!("cargo:rerun-if-changed=src/macos/shell.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new().file("src/macos/shell.m").flag("-fobjc-arc").flag("-fmodules").compile("t3a_shell");
    for framework in ["AppKit", "QuartzCore", "ServiceManagement", "UserNotifications"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
