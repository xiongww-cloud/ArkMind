fn main() {
    // macOS 特殊依赖
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=dylib=c++");
    }

    tauri_build::build()
}
