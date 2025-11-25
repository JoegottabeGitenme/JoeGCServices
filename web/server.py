#!/usr/bin/env python3
"""
Simple HTTP server for the WMS Dashboard
Run with: python3 server.py
Then visit: http://localhost:8000
"""

import http.server
import socketserver
import os
from pathlib import Path

PORT = 8000
HANDLER = http.server.SimpleHTTPRequestHandler


class MyHTTPRequestHandler(HANDLER):
    def end_headers(self):
        # Add CORS headers
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-type")
        super().end_headers()

    def do_OPTIONS(self):
        self.send_response(200)
        self.end_headers()


if __name__ == "__main__":
    # Change to web directory
    web_dir = Path(__file__).parent
    os.chdir(web_dir)

    with socketserver.TCPServer(("", PORT), MyHTTPRequestHandler) as httpd:
        print(f"🚀 WMS Dashboard running at http://localhost:{PORT}")
        print(f"📁 Serving from: {web_dir}")
        print(f"🛑 Press Ctrl+C to stop")
        httpd.serve_forever()
