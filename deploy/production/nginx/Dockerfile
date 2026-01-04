FROM nginx:1.25-alpine

# Install certbot and required tools
RUN apk add --no-cache \
    certbot \
    certbot-nginx \
    openssl \
    apache2-utils \
    curl \
    bash


# Copy nginx config (will be replaced during deployment with generated version)
COPY nginx.conf.template /etc/nginx/nginx.conf.template

# Default config that works before TLS is set up (HTTP only)
RUN echo 'worker_processes auto; \
events { worker_connections 1024; } \
http { \
    server { \
        listen 80; \
        server_name _; \
        location /.well-known/acme-challenge/ { root /var/www/certbot; } \
        location /health { return 200 "healthy\n"; add_header Content-Type text/plain; } \
        location / { return 503 "TLS not configured yet\n"; } \
    } \
}' > /etc/nginx/nginx.conf

EXPOSE 80 443

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost/health || exit 1

CMD ["nginx", "-g", "daemon off;"]
