const fs = require('fs');
let buf = fs.readFileSync('src-tauri/src/commands/kb_commands.rs');

// Read the file and replace ALL non-ASCII bytes with ASCII equivalents
// This is a nuclear option but should fix the encoding
let result = [];
for (let i = 0; i < buf.length; i++) {
    const byte = buf[i];
    // CR, LF, TAB: keep as-is
    if (byte === 0x0A || byte === 0x0D || byte === 0x09) {
        result.push(byte);
        continue;
    }
    // Standard ASCII printable (space through ~): keep
    if (byte >= 0x20 && byte <= 0x7E) {
        result.push(byte);
        continue;
    }
    // Multi-byte UTF-8 sequences: decode and re-encode
    if (byte >= 0xC0 && byte <= 0xDF) {
        // 2-byte UTF-8
        if (i + 1 < buf.length) {
            const code = ((byte & 0x1F) << 6) | (buf[i+1] & 0x3F);
            if (code >= 0x4E00 && code <= 0x9FFF) {
                // Chinese char - keep as UTF-8
                result.push(byte, buf[i+1]);
            }
            i++;
        }
        continue;
    }
    if (byte >= 0xE0 && byte <= 0xEF) {
        // 3-byte UTF-8
        if (i + 2 < buf.length) {
            const code = ((byte & 0x0F) << 12) | ((buf[i+1] & 0x3F) << 6) | (buf[i+2] & 0x3F);
            if ((code >= 0x4E00 && code <= 0x9FFF) || (code >= 0x3000 && code <= 0x303F)) {
                // Chinese or CJK punctuation - keep
                result.push(byte, buf[i+1], buf[i+2]);
            } else if (code >= 0xFF00 && code <= 0xFFEF) {
                // Fullwidth → normal-width
                const ascii = code - 0xFEE0;
                result.push(ascii);
            }
            i += 2;
        }
        continue;
    }
    // For anything else (non-ASCII, non-Chinese), skip
}

fs.writeFileSync('src-tauri/src/commands/kb_commands.rs', Buffer.from(result));
console.log('Done. Output size:', result.length);
