const fs = require('fs');
let c = fs.readFileSync('src-tauri/src/commands/kb_commands.rs', 'utf8');

// Fix agent_chat_workflow call: old version has 2 params (no history), but also no history was passed
// Actually the old agent_workflow doesn't have history param. Let's use 2 param form.
c = c.replace(/agent_chat_workflow\(query, max_iter, history\)/g, 'agent_chat_workflow(query, max_iter)');

// Fix build_chunk_metadata: the old version in agent_workflow has 2 params (st, sid)
// But the new kb_commands passes 5 params. The latest agent_workflow was reverted.
// Need to check what the actual function accepts.
// From the file: pub fn build_chunk_metadata(source_type: &str, source_id: &str) -> String
// So it only takes 2 params. Remove the extra 3.
c = c.replace(/build_chunk_navigation_card\(st, sid, chunk_id, chunk_index, total_parents\)/g, 'build_chunk_metadata(st, sid)');

fs.writeFileSync('src-tauri/src/commands/kb_commands.rs', c, 'utf8');
console.log('Compatibility fixes applied');
