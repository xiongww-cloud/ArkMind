fn main() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bin_dir = manifest_dir.join("bin");

    // 静态链接 pdfium
    println!("cargo:rustc-link-lib=static=pdfium");
    println!("cargo:rustc-link-search=native={}", bin_dir.display());

    // macOS 特殊依赖
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=dylib=c++");
    }

    tauri_build::build()
}
