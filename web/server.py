#!/usr/bin/env python3
"""
Simple HTTP server for the WMS Dashboard
Run with: python3 server.py
Then visit: http://localhost:8000
"""

import http.server
import socket
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


class DualStackTCPServer(socketserver.TCPServer):
    """TCP Server that supports both IPv4 and IPv6."""

    # Allow IPv6 if available
    address_family = socket.AF_INET6

    def server_bind(self):
        # Enable dual-stack (IPv4 + IPv6) on the socket
        # IPV6_V6ONLY=False allows the socket to accept both IPv4 and IPv6
        try:
            self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        except (AttributeError, OSError):
            # Fall back to IPv4-only if dual-stack is not supported
            self.address_family = socket.AF_INET
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        super().server_bind()


if __name__ == "__main__":
    # Change to web directory
    web_dir = Path(__file__).parent
    os.chdir(web_dir)

    # Try dual-stack first, fall back to IPv4
    try:
        httpd = DualStackTCPServer(("::", PORT), MyHTTPRequestHandler)
        print(f"🚀 WMS Dashboard running at http://localhost:{PORT} (IPv4+IPv6)")
    except OSError:
        # Fall back to IPv4 only
        httpd = socketserver.TCPServer(("", PORT), MyHTTPRequestHandler)
        print(f"🚀 WMS Dashboard running at http://localhost:{PORT} (IPv4 only)")

    print(f"📁 Serving from: {web_dir}")
    print(f"🛑 Press Ctrl+C to stop")

    try:
        httpd.serve_forever()
    finally:
        httpd.server_close()
