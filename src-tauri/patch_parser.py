import sys
sys.stdout.reconfigure(encoding='utf-8')

with open('src/services/file_parser.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

new_funcs = """// mutool per-page text fallback
fn try_mutool_text_per_page(path: &str, page_count: usize) -> Option<Vec<String>> {
    let cmd = find_mutool_binary()?;
    let output = std::process::Command::new(&cmd)
        .args(["draw", "-F", "text", path])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let pages: Vec<&str> = raw.split('\x0c').collect();
    let mut result: Vec<String> = pages.iter().map(|p| p.trim().to_string()).collect();
    while result.len() < page_count { result.push(String::new()); }
    Some(result)
}

fn has_real_text(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_alphanumeric() || c as u32 > 127)
}

"""

# Insert after line 506 (the '}' closing try_ocr_pdf)
new_lines = lines[:507] + ['\n'] + [new_funcs] + lines[507:]

with open('src/services/file_parser.rs', 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print('Patch applied successfully')
print(f'File now has {len(new_lines)} lines')
