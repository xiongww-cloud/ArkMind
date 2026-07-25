
/// Fallback: parse HTML-style XML tool calls from content
/// Format: tool_call / function=name / parameter=name value
fn parse_html_tool_calls(content: &str) -> Option<Vec<Value>> {
    let tc_open = "\u{3c}tool_call\u{3e}";
    let tc_close = "\u{3c}/tool_call\u{3e}";
    let fn_open = "\u{3c}function=";
    let param_open = "\u{3c}parameter=";
    let param_close = "\u{3c}/parameter\u{3e}";

    let mut tool_calls = Vec::new();
    let mut remaining = content;
    while let Some(tc_start) = remaining.find(tc_open) {
        let after_tag = tc_start + tc_open.len();
        let tc_end = remaining[after_tag..].find(tc_close).unwrap_or(remaining[after_tag..].len());
        let block = &remaining[after_tag..after_tag + tc_end];
        remaining = &remaining[after_tag + tc_end + tc_close.len()..];

        let func_name = if let Some(fn_start) = block.find(fn_open) {
            let ns = fn_start + fn_open.len();
            let ne = block[ns..].find('>').map(|i| ns + i).unwrap_or(ns);
            block[ns..ne].trim().to_string()
        } else {
            continue;
        };
        if func_name.is_empty() { continue; }

        let mut params = serde_json::Map::new();
        let mut inner = block;
        while let Some(p_start) = inner.find(param_open) {
            let pn_start = p_start + param_open.len();
            let pn_end = inner[pn_start..].find('>').map(|i| pn_start + i).unwrap_or(pn_start);
            let pname = inner[pn_start..pn_end].trim().to_string();
            let after_p = pn_end + 1;
            let p_end = inner[after_p..].find(param_close).unwrap_or(inner[after_p..].len());
            let pval = inner[after_p..after_p + p_end].trim().to_string();
            let jv = serde_json::from_str::<Value>(&pval).unwrap_or_else(|_| json!(pval));
            params.insert(pname, jv);
            inner = &inner[after_p + p_end + param_close.len()..];
        }
        tool_calls.push(json!({
            "id": format!("call_{}", &uuid::Uuid::new_v4().to_string()[..8]),
            "type": "function",
            "function": {"name": func_name, "arguments": serde_json::to_string(&Value::Object(params)).unwrap_or_default()}
        }));
    }
    if tool_calls.is_empty() { None } else { Some(tool_calls) }
}
