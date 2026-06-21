const fs = require('fs');
let c = fs.readFileSync('src-tauri/src/services/agent_workflow.rs', 'utf8');

// Find the duplicate tools block: it starts after "],)\n}}" and goes until the next "],)\n}"
const dupStart = c.indexOf('}},');
const dupEnd = c.indexOf(']\n)', dupStart) + 3;
const afterDup = c.indexOf('}', dupEnd) + 1;
// Remove the duplicate block (from dupStart to afterDup)
c = c.slice(0, dupStart - 1) + c.slice(afterDup);
fs.writeFileSync('src-tauri/src/services/agent_workflow.rs', c, 'utf8');
console.log('Done');
