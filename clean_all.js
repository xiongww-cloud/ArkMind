const fs = require('fs');
let c = fs.readFileSync('src-tauri/src/commands/kb_commands.rs', 'utf8');

// Remove all non-ASCII characters that could cause issues
// Keep only ASCII printable (0x20-0x7E) plus newline/tab
let cleaned = '';
for (let i = 0; i < c.length; i++) {
    const code = c.charCodeAt(i);
    if (code === 10 || code === 13 || code === 9 || code === 0) {
        cleaned += c[i];
    } else if (code >= 0x20 && code <= 0x7E) {
        cleaned += c[i];
    } else if (code >= 0x4E00 && code <= 0x9FFF) {
        // Chinese characters - keep
        cleaned += c[i];
    } else if (code >= 0x3000 && code <= 0x303F) {
        // CJK punctuation - keep
        cleaned += c[i];
    } else if (code >= 0xFF00 && code <= 0xFFEF) {
        // Fullwidth forms - replace with ASCII equivalents
        cleaned += String.fromCharCode(code - 0xFEE0);
    }
}
console.log('Cleaned size:', cleaned.length, 'from', c.length);

// Check for any problematic backslash positions
const lines = cleaned.split('\n');
for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    // Check for backslash not in string context
    // Just warn about lines with backslash that look suspicious
    if (l.includes('\\') && !l.includes('"\\') && !l.includes("'\\")) {
        // Check if it's in a string
        const inString = (l.match(/"/g) || []).length % 2 === 1;
        if (inString && !l.includes('"\\') && !l.includes("'\\")) {
            console.log('Line', i+1, 'has suspicious backslash');
        }
    }
}

fs.writeFileSync('src-tauri/src/commands/kb_commands.rs', cleaned, 'utf8');
console.log('Written clean file');
