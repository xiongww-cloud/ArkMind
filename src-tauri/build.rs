fn main() {
    // ★ 静态链接 pdfium：PDFIUM_STATIC_LIB_PATH 会被 pdfium-render 的 build.rs 读取
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bin_dir = manifest_dir.join("bin");
    std::env::set_var("PDFIUM_STATIC_LIB_PATH", &bin_dir);
    tauri_build::build()
}
