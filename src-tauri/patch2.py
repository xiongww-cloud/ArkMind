with open('src/services/file_parser.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

# new try_ocr_pdf with per-page mutool fallback
new_func = '''fn try_ocr_pdf(path: &str) -> String {
    // 先检查视觉模型是否已配置，避免没配时逐页等待超时
    if get_vision_config().is_none() {
        eprintln!("[文件解析] 未配置视觉模型，跳过 OCR");
        return String::new();
    }

    let uris = match pdf_to_images_base64(path) {
        Some(u) => u,
        None => {
            eprintln!("[文件解析] PDF 渲染失败，无法 OCR: {}", path);
            return String::new();
        }
    };

    // 预先用 mutool 提取纯文本作为兜底
    let mutool_pages = try_mutool_text_per_page(path, uris.len());
    if mutool_pages.is_some() {
        eprintln!("[文件解析] mutool 已提取纯文本，作为 OCR 兜底");
    }

    eprintln!("[文件解析] OCR 开始处理 {} 页...", uris.len());
    let mut full_text = String::new();
    for (i, uri) in uris.iter().enumerate() {
        eprintln!("[文件解析] OCR 第 {} 页 / 共 {} 页...", i + 1, uris.len());
        let cropped = crop_image_uri(uri, 0.08, 0.08).unwrap_or_else(|| uri.clone());
        let ocr_text = ocr_image_sync(&cropped);

        // 检查 OCR 结果是否有真实文本
        let page_text = if has_real_text(&ocr_text) {
            ocr_text
        } else if let Some(ref fallback) = mutool_pages {
            let fb = fallback.get(i).map(|s| s.as_str()).unwrap_or("");
            if !fb.is_empty() {
                eprintln!("[文件解析] OCR 第{}页无真实文本，用 mutool 兜底", i + 1);
                fb.to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !page_text.is_empty() {
            if !full_text.is_empty() {
                full_text.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1));
            }
            full_text.push_str(&page_text);
        }
    }
    full_text
}
'''

# Replace old try_ocr_pdf (lines 500-529 0-indexed = 499-529)
new_lines = lines[:499] + [new_func] + lines[530:]

with open('src/services/file_parser.rs', 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print('Patched try_ocr_pdf with per-page fallback')
