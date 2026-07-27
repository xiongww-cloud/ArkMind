#!/usr/bin/env python3
"""
ArkMind Context Debug Server
用于调试 agent_workflow 发送给 LLM 的完整上下文

使用方法：
1. 启动服务器：python3 debug_server.py
2. 浏览器自动打开 http://localhost:9876
3. 在 ArkMind 中执行对话
4. 刷新浏览器查看 context
"""

import http.server
import json
import os
import webbrowser
import threading
import time
from pathlib import Path

PORT = 9876
DEBUG_DIR = Path.home() / ".arkmind" / "debug"
CONTEXT_FILE = DEBUG_DIR / "context.json"

HTML_PAGE = """<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <title>ArkMind Context Debug</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; background: #f5f5f5; padding: 20px; }
        h1 { color: #333; margin-bottom: 20px; }
        .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px; }
        .stats { color: #666; font-size: 14px; }
        .refresh-btn { background: #007AFF; color: white; border: none; padding: 8px 16px; border-radius: 6px; cursor: pointer; }
        .refresh-btn:hover { background: #0056CC; }
        .message { background: white; border-radius: 8px; margin-bottom: 12px; padding: 16px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
        .message-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
        .role { font-weight: 600; padding: 4px 12px; border-radius: 4px; font-size: 12px; text-transform: uppercase; }
        .role-system { background: #E5E5EA; color: #333; }
        .role-user { background: #007AFF; color: white; }
        .role-assistant { background: #34C759; color: white; }
        .role-tool { background: #FF9500; color: white; }
        .content { white-space: pre-wrap; word-break: break-word; font-size: 14px; line-height: 1.6; }
        .tool-calls { background: #F8F9FA; border-radius: 4px; padding: 12px; margin-top: 8px; font-size: 13px; }
        .tool-results { background: #FFF3CD; border-radius: 4px; padding: 12px; margin-top: 8px; font-size: 13px; }
        .collapsed { max-height: 100px; overflow: hidden; }
        .expand-btn { color: #007AFF; cursor: pointer; font-size: 12px; margin-top: 4px; }
        .json-view { background: #1E1E1E; color: #D4D4D4; padding: 16px; border-radius: 8px; overflow-x: auto; font-family: 'Monaco', 'Menlo', monospace; font-size: 12px; white-space: pre-wrap; }
        .tab-container { display: flex; gap: 8px; margin-bottom: 16px; }
        .tab { padding: 8px 16px; background: white; border: 1px solid #ddd; border-radius: 6px; cursor: pointer; }
        .tab.active { background: #007AFF; color: white; border-color: #007AFF; }
        .tab-content { display: none; }
        .tab-content.active { display: block; }
    </style>
</head>
<body>
    <div class="header">
        <h1>ArkMind Context Debug</h1>
        <div>
            <span class="stats" id="stats">Loading...</span>
            <button class="refresh-btn" onclick="loadContext()">刷新</button>
        </div>
    </div>
    
    <div class="tab-container">
        <div class="tab active" onclick="showTab('visual')">可视化</div>
        <div class="tab" onclick="showTab('json')">JSON</div>
    </div>
    
    <div id="visual" class="tab-content active"></div>
    <div id="json" class="tab-content"><pre class="json-view" id="json-view"></pre></div>

    <script>
        let currentData = null;
        
        function showTab(name) {
            document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            document.querySelector(`.tab:${name === 'visual' ? 'first-child' : 'last-child'}`).classList.add('active');
            document.getElementById(name).classList.add('active');
        }
        
        async function loadContext() {
            try {
                const r = await fetch('/context');
                if (!r.ok) throw new Error('No context available');
                currentData = await r.json();
                renderVisual(currentData);
                renderJson(currentData);
                document.getElementById('stats').textContent = 
                    `${currentData.length} 条消息 | 最后更新: ${new Date().toLocaleTimeString()}`;
            } catch (e) {
                document.getElementById('visual').innerHTML = 
                    '<p style="color:red">无法加载 context 数据。请确保 ArkMind 正在运行。</p>';
            }
        }
        
        function renderVisual(data) {
            const container = document.getElementById('visual');
            container.innerHTML = data.map((msg, i) => {
                const role = msg.role || 'unknown';
                const content = msg.content || '';
                const toolCalls = msg.tool_calls;
                const toolResults = msg.tool_results;
                
                let html = `<div class="message">
                    <div class="message-header">
                        <span class="role role-${role}">${role}</span>
                        <span style="color:#999;font-size:12px">#${i}</span>
                    </div>
                    <div class="content">${escapeHtml(content)}</div>`;
                
                if (toolCalls && toolCalls.length > 0) {
                    html += `<div class="tool-calls">
                        <strong>Tool Calls:</strong>
                        <pre>${escapeHtml(JSON.stringify(toolCalls, null, 2))}</pre>
                    </div>`;
                }
                
                if (toolResults && toolResults.length > 0) {
                    html += `<div class="tool-results">
                        <strong>Tool Results:</strong>
                        <pre>${escapeHtml(JSON.stringify(toolResults, null, 2))}</pre>
                    </div>`;
                }
                
                html += '</div>';
                return html;
            }).join('');
        }
        
        function renderJson(data) {
            document.getElementById('json-view').textContent = JSON.stringify(data, null, 2);
        }
        
        function escapeHtml(text) {
            const div = document.createElement('div');
            div.textContent = text;
            return div.innerHTML;
        }
        
        loadContext();
        setInterval(loadContext, 5000);
    </script>
</body>
</html>
"""


class DebugHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/' or self.path == '/index.html':
            self.send_response(200)
            self.send_header('Content-type', 'text/html; charset=utf-8')
            self.end_headers()
            self.wfile.write(HTML_PAGE.encode('utf-8'))
        elif self.path == '/context':
            self.send_response(200)
            self.send_header('Content-type', 'application/json; charset=utf-8')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            try:
                data = CONTEXT_FILE.read_text(encoding='utf-8')
                self.wfile.write(data.encode('utf-8'))
            except FileNotFoundError:
                self.wfile.write(b'[]')
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, format, *args):
        pass  # 静默日志


def open_browser():
    time.sleep(1)
    webbrowser.open(f'http://localhost:{PORT}')


if __name__ == '__main__':
    os.makedirs(DEBUG_DIR, exist_ok=True)
    print(f"ArkMind Debug Server started at http://localhost:{PORT}")
    print(f"Debug directory: {DEBUG_DIR}")
    print(f"Context file: {CONTEXT_FILE}")
    print("Press Ctrl+C to stop")
    
    threading.Thread(target=open_browser, daemon=True).start()
    
    server = http.server.HTTPServer(('127.0.0.1', PORT), DebugHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nServer stopped")
        server.server_close()
