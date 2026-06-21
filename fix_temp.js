const fs = require('fs');
let c = fs.readFileSync('src-tauri/src/commands/kb_commands.rs', 'utf8');

// Fix the specific corrupted format strings at lines ~222, 249, 259, 261
// The issue is that `\n` sequences got mangled by Set-Content encoding
// Let's fix known patterns

// Fix "文件不存?}" -> closed properly
c = c.replace(/文件不存\}"/, '文件不存在"}');

// Fix the format strings that contain corrupted \n
// These are lines like: format!("\n\n--- [父块{}...
// The backslashes in front of n were corrupted

// Check for specific patterns and fix
c = c.replace(/未知开始标记/, 'unknown');  // Not needed

fs.writeFileSync('src-tauri/src/commands/kb_commands.rs', c, 'utf8');
console.log('Done');
