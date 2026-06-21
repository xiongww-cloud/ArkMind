#!/usr/bin/env python3
"""Arkmind 许可证生成器 — 可选本地 HTTP 服务器（双击 index.html 亦可直接使用）"""
import http.server
import webbrowser
import os

PORT = 8765
DIR = os.path.dirname(os.path.abspath(__file__))

class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=DIR, **kwargs)

print(f'Arkmind 许可证生成器')
print(f'  地址: http://localhost:{PORT}')
print(f'  按 Ctrl+C 停止')
webbrowser.open(f'http://localhost:{PORT}')

with http.server.HTTPServer(('', PORT), Handler) as httpd:
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print('\n已停止')
