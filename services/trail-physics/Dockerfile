# trail-physics service
#
# The first Python service with real third-party dependencies in this
# codebase (web-dashboard uses a bare python:3.11-slim + stdlib only, no
# Dockerfile needed -- see docker-compose.yml). numpy/scipy/zarr/s3fs/
# psycopg need an actual build step, hence this Dockerfile.
#
# Build:
#   docker build -f services/trail-physics/Dockerfile -t trail-physics services/trail-physics
FROM python:3.12-slim

WORKDIR /app

# psycopg[binary] avoids needing libpq-dev at build time; kept minimal
# deliberately since this service has no GDAL/WhiteboxTools dependency --
# those belong to pipelines/static/ (run offline, not in this container).
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY . .

ENV PYTHONUNBUFFERED=1

CMD ["python3", "main.py"]
