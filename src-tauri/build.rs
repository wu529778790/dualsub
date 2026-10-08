fn main() {
    // spike：链接系统 Homebrew 的 libmpv（仅 macOS）
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
        println!("cargo:rustc-link-lib=dylib=mpv");
    }
    tauri_build::build()
}
