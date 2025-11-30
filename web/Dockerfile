FROM python:3.11-slim

WORKDIR /app

# Copy web files
COPY index.html app.js style.css server.py ./

EXPOSE 8000

CMD ["python3", "server.py"]
