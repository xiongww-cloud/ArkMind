const fs = require('fs');
let c = fs.readFileSync('src-tauri/src/commands/kb_commands.rs', 'utf8');

// Replace the entire ext_tool_search_knowledge_base function with a clean version
// Fix line 222: the format!("\"...\""...) is valid Rust but somehow parser chokes
// Let's just replace that specific format call

// Find and fix the problematic line
const oldFtsQuery = `    let fts_query = keywords.iter().map(|k| format!("\"{}\"", k.replace('"', ""))).collect::<Vec<_>>().join(" OR ");`;
const newFtsQuery = `    let fts_query: Vec<String> = keywords.iter().map(|k| format!("\"{}\"", k.replace('"', ""))).collect();
    let fts_query_str = fts_query.join(" OR ");`;

c = c.replace(oldFtsQuery, newFtsQuery);

// Also fix format strings at lines 259, 261 - write them clean
// They have format!("\n\n--- [父块{}...])
// Replace with equivalent using raw strings
const oldFormat1 = `format!("\n\n--- [父块{}|RRF={:.3}] ---\n{}", count, rrf_score, t)`;
const newFormat1 = `format!("\\n\\n--- [父块{}|RRF={:.3}] ---\\n{}", count, rrf_score, t)`;
c = c.replace(oldFormat1, newFormat1);

const oldFormat2 = `format!("\n\n--- [父块{}|RRF={:.3}] ---\n{}\n--------\n{}", count, rrf_score, meta, t)`;
const newFormat2 = `format!("\\n\\n--- [父块{}|RRF={:.3}] ---\\n{}\\n--------\\n{}", count, rrf_score, meta, t)`;
c = c.replace(oldFormat2, newFormat2);

// Fix fts5_search_chunks references
c = c.replace(/fts5_search_chunks_slim/g, 'fts5_search_chunks');

fs.writeFileSync('src-tauri/src/commands/kb_commands.rs', c, 'utf8');
console.log('Cleaned and fixed. Done');
