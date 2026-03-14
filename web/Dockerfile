FROM python:3.11-slim

WORKDIR /app

# Copy all web files (HTML pages, JS, CSS, assets)
COPY . ./

EXPOSE 8000

CMD ["python3", "server.py"]
