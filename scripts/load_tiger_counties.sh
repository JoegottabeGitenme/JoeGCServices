#!/bin/bash
# Load Census TIGER/Line county polygons into the `tiger_counties` PostGIS table.
#
# This is a ONE-OFF reference-data load. It downloads the national county
# shapefile from the Census Bureau and imports it via ogr2ogr (bundled in the
# postgis/postgis Docker image). After loading, storm-event ingestion can stamp
# `county_fips` via spatial join and the EDR /counties endpoint can serve
# boundary geometry.
#
# Usage:
#   # Run against the docker-compose postgres container (default):
#   ./scripts/load_tiger_counties.sh
#
#   # Or against an arbitrary database:
#   DATABASE_URL="postgresql://user:pass@host:5432/db" ./scripts/load_tiger_counties.sh
#
# Requirements: docker (to exec into the postgis container) OR a local ogr2ogr
# with PostgreSQL driver. Set USE_LOCAL_OGR=1 to use a local ogr2ogr.

set -euo pipefail

# -----------------------------------------------------------------------------
# Configuration
# -----------------------------------------------------------------------------
TIGER_YEAR="${TIGER_YEAR:-2023}"
TIGER_URL="https://www2.census.gov/geo/tiger/TIGER${TIGER_YEAR}/COUNTY/tl_${TIGER_YEAR}_us_county.zip"

POSTGRES_CONTAINER="${POSTGRES_CONTAINER:-weather-wms-postgres-1}"
DATABASE_URL="${DATABASE_URL:-postgresql://weatherwms:weatherwms@postgres:5432/weatherwms}"

WORK_DIR="${WORK_DIR:-/tmp/tiger_counties}"
ZIP_PATH="${WORK_DIR}/tl_${TIGER_YEAR}_us_county.zip"
SHP_NAME="tl_${TIGER_YEAR}_us_county.shp"

# Target table. We import to a staging table then normalize column names so the
# final `tiger_counties` matches the schema in STORM_EVENTS_SCHEMA_SQL.
STAGING_TABLE="tiger_counties_staging"

echo "==> TIGER county load (year ${TIGER_YEAR})"
mkdir -p "${WORK_DIR}"

# -----------------------------------------------------------------------------
# 1. Download + unzip the shapefile
# -----------------------------------------------------------------------------
if [ ! -f "${ZIP_PATH}" ]; then
    echo "==> Downloading ${TIGER_URL}"
    curl -fSL --retry 3 -o "${ZIP_PATH}" "${TIGER_URL}"
else
    echo "==> Using cached ${ZIP_PATH}"
fi

echo "==> Unzipping"
( cd "${WORK_DIR}" && unzip -o "${ZIP_PATH}" >/dev/null )

if [ ! -f "${WORK_DIR}/${SHP_NAME}" ]; then
    echo "ERROR: expected ${WORK_DIR}/${SHP_NAME} not found after unzip" >&2
    exit 1
fi

# -----------------------------------------------------------------------------
# 2. Import via ogr2ogr
# -----------------------------------------------------------------------------
# TIGER county attributes of interest:
#   GEOID    -> 5-digit county FIPS (state + county)
#   NAME     -> county name
#   STATEFP  -> 2-digit state FIPS
# State abbreviation is not in the county shapefile; we derive it from STATEFP
# via a lookup table after import.

run_ogr2ogr() {
    # $1 = connection string usable from wherever ogr2ogr runs
    ogr2ogr \
        -f PostgreSQL \
        "PG:$1" \
        "$2" \
        -nln "${STAGING_TABLE}" \
        -nlt MULTIPOLYGON \
        -t_srs EPSG:4326 \
        -lco GEOMETRY_NAME=geom \
        -lco FID=ogc_fid \
        -overwrite \
        -progress
}

if [ "${USE_LOCAL_OGR:-0}" = "1" ]; then
    echo "==> Importing with local ogr2ogr"
    run_ogr2ogr "${DATABASE_URL}" "${WORK_DIR}/${SHP_NAME}"
elif command -v ogr2ogr >/dev/null 2>&1; then
    echo "==> Importing with host ogr2ogr"
    run_ogr2ogr "${DATABASE_URL}" "${WORK_DIR}/${SHP_NAME}"
elif docker image inspect ghcr.io/osgeo/gdal:ubuntu-small-latest >/dev/null 2>&1 || \
     docker pull ghcr.io/osgeo/gdal:ubuntu-small-latest >/dev/null 2>&1; then
    echo "==> Importing via gdal Docker image"
    docker run --rm \
        --network host \
        -v "${WORK_DIR}:/work:ro" \
        ghcr.io/osgeo/gdal:ubuntu-small-latest \
        ogr2ogr \
            -f PostgreSQL \
            "PG:${DATABASE_URL}" \
            "/work/${SHP_NAME}" \
            -nln "${STAGING_TABLE}" \
            -nlt MULTIPOLYGON \
            -t_srs EPSG:4326 \
            -lco GEOMETRY_NAME=geom \
            -lco FID=ogc_fid \
            -overwrite \
            -progress
else
    echo "==> Importing via docker container ${POSTGRES_CONTAINER}"
    # Copy the shapefile parts into the container, then run ogr2ogr there.
    # NOTE: postgis/postgis base image does NOT include ogr2ogr/GDAL by default.
    # If this fails, use: USE_LOCAL_OGR=1 ./scripts/load_tiger_counties.sh
    # or: apt-get install -y gdal-bin && USE_LOCAL_OGR=1 ...
    docker exec "${POSTGRES_CONTAINER}" mkdir -p /tmp/tiger
    for ext in shp shx dbf prj; do
        if [ -f "${WORK_DIR}/tl_${TIGER_YEAR}_us_county.${ext}" ]; then
            docker cp "${WORK_DIR}/tl_${TIGER_YEAR}_us_county.${ext}" \
                "${POSTGRES_CONTAINER}:/tmp/tiger/tl_${TIGER_YEAR}_us_county.${ext}"
        fi
    done
    # Inside the container, postgres is reachable on localhost.
    CONTAINER_DB_URL="${CONTAINER_DB_URL:-postgresql://weatherwms:weatherwms@localhost:5432/weatherwms}"
    docker exec "${POSTGRES_CONTAINER}" ogr2ogr \
        -f PostgreSQL \
        "PG:${CONTAINER_DB_URL}" \
        "/tmp/tiger/${SHP_NAME}" \
        -nln "${STAGING_TABLE}" \
        -nlt MULTIPOLYGON \
        -t_srs EPSG:4326 \
        -lco GEOMETRY_NAME=geom \
        -lco FID=ogc_fid \
        -overwrite \
        -progress
fi

# -----------------------------------------------------------------------------
# 3. Normalize staging -> tiger_counties
# -----------------------------------------------------------------------------
# Run SQL inside the container against localhost.
PSQL_DB_URL="${CONTAINER_DB_URL:-postgresql://weatherwms:weatherwms@localhost:5432/weatherwms}"

run_psql() {
    if [ "${USE_LOCAL_OGR:-0}" = "1" ]; then
        psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -c "$1"
    else
        docker exec -i "${POSTGRES_CONTAINER}" psql "${PSQL_DB_URL}" -v ON_ERROR_STOP=1 -c "$1"
    fi
}

echo "==> Ensuring tiger_counties table exists"
run_psql "CREATE EXTENSION IF NOT EXISTS postgis;
CREATE TABLE IF NOT EXISTS tiger_counties (
    geoid CHAR(5) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    state_fips CHAR(2),
    state_abbr CHAR(2),
    geom GEOMETRY(MultiPolygon, 4326) NOT NULL
);"

echo "==> Populating tiger_counties from staging"
run_psql "
INSERT INTO tiger_counties (geoid, name, state_fips, state_abbr, geom)
SELECT
    s.geoid,
    s.name,
    s.statefp,
    fips.abbr,
    ST_Multi(ST_SetSRID(s.geom, 4326))
FROM ${STAGING_TABLE} s
LEFT JOIN (VALUES
    ('01','AL'),('02','AK'),('04','AZ'),('05','AR'),('06','CA'),('08','CO'),
    ('09','CT'),('10','DE'),('11','DC'),('12','FL'),('13','GA'),('15','HI'),
    ('16','ID'),('17','IL'),('18','IN'),('19','IA'),('20','KS'),('21','KY'),
    ('22','LA'),('23','ME'),('24','MD'),('25','MA'),('26','MI'),('27','MN'),
    ('28','MS'),('29','MO'),('30','MT'),('31','NE'),('32','NV'),('33','NH'),
    ('34','NJ'),('35','NM'),('36','NY'),('37','NC'),('38','ND'),('39','OH'),
    ('40','OK'),('41','OR'),('42','PA'),('44','RI'),('45','SC'),('46','SD'),
    ('47','TN'),('48','TX'),('49','UT'),('50','VT'),('51','VA'),('53','WA'),
    ('54','WV'),('55','WI'),('56','WY'),('60','AS'),('66','GU'),('69','MP'),
    ('72','PR'),('78','VI')
) AS fips(fp, abbr) ON fips.fp = s.statefp
ON CONFLICT (geoid) DO UPDATE SET
    name = EXCLUDED.name,
    state_fips = EXCLUDED.state_fips,
    state_abbr = EXCLUDED.state_abbr,
    geom = EXCLUDED.geom;"

echo "==> Creating indexes"
run_psql "CREATE INDEX IF NOT EXISTS idx_tiger_counties_geom ON tiger_counties USING GIST(geom);
CREATE INDEX IF NOT EXISTS idx_tiger_counties_state ON tiger_counties(state_abbr);"

echo "==> Dropping staging table"
run_psql "DROP TABLE IF EXISTS ${STAGING_TABLE};"

echo "==> Done. Row count:"
run_psql "SELECT COUNT(*) AS counties FROM tiger_counties;"
