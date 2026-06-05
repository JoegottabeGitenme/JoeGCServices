#!/bin/bash
# Load Census TIGER county polygons into tiger_counties using Python + psql only.
# No ogr2ogr / GDAL required — uses the Census Bureau's Cartographic Boundary
# GeoJSON files (5m resolution, good for display) fetched directly via HTTPS.
#
# Downloads ~6 MB of GeoJSON (all US counties), then bulk-loads via psql COPY.
#
# Usage:
#   DATABASE_URL=postgresql://weatherwms:weatherwms@localhost:5432/weatherwms \
#   ./scripts/load_tiger_counties_py.sh

set -euo pipefail

DATABASE_URL="${DATABASE_URL:-postgresql://weatherwms:weatherwms@localhost:5432/weatherwms}"
TIGER_YEAR="${TIGER_YEAR:-2023}"
GEOJSON_URL="https://www2.census.gov/geo/tiger/GENZ${TIGER_YEAR}/shp/cb_${TIGER_YEAR}_us_county_5m.zip"
WORK_DIR="${WORK_DIR:-/tmp/tiger_counties_py}"

mkdir -p "${WORK_DIR}"
ZIPFILE="${WORK_DIR}/counties_5m.zip"
GEOJSON="${WORK_DIR}/counties.geojson"

echo "==> TIGER county load via Python (year ${TIGER_YEAR}, 5m cartographic)"

# ─── Download ─────────────────────────────────────────────────────────────────
if [ ! -f "${ZIPFILE}" ]; then
    echo "==> Downloading ${GEOJSON_URL}"
    curl -fSL --retry 3 -o "${ZIPFILE}" "${GEOJSON_URL}"
else
    echo "==> Using cached ${ZIPFILE}"
fi

# Convert shapefile → GeoJSON using Python's shapefile lib if available,
# or use ogr2ogr / GDAL Docker if we have it. Fall through to the psql approach.
echo "==> Converting shapefile → GeoJSON"
python3 - <<'PYEOF'
import os, sys, zipfile, json

work  = os.environ.get("WORK_DIR", "/tmp/tiger_counties_py")
year  = os.environ.get("TIGER_YEAR", "2023")
zpath = os.path.join(work, "counties_5m.zip")
out   = os.path.join(work, "counties.geojson")

if os.path.exists(out):
    print(f"  reusing {out}")
    sys.exit(0)

# Try pyshp (shapefile) if available
try:
    import shapefile  # pip install pyshp
    with zipfile.ZipFile(zpath) as z:
        z.extractall(work)

    shp_name = None
    for f in os.listdir(work):
        if f.endswith(".shp") and "county" in f.lower():
            shp_name = os.path.join(work, f)
            break

    if not shp_name:
        raise FileNotFoundError("No county .shp found in zip")

    sf = shapefile.Reader(shp_name)
    fields = [f[0] for f in sf.fields[1:]]
    features = []
    for sr in sf.shapeRecords():
        props = dict(zip(fields, sr.record))
        geoid   = str(props.get("GEOID","")).zfill(5)
        name    = str(props.get("NAME",""))
        statefp = str(props.get("STATEFP",""))
        geom    = sr.shape.__geo_interface__
        features.append({
            "type":"Feature",
            "properties":{"GEOID":geoid,"NAME":name,"STATEFP":statefp},
            "geometry": geom,
        })
    with open(out, "w") as fh:
        json.dump({"type":"FeatureCollection","features":features}, fh)
    print(f"  wrote {len(features)} counties to {out}")
    sys.exit(0)
except ImportError:
    pass  # pyshp not available; fall through

# Fallback: try to install pyshp on the fly
import subprocess
try:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "pyshp"], stderr=subprocess.DEVNULL)
    import shapefile
    with zipfile.ZipFile(zpath) as z:
        z.extractall(work)
    shp_name = None
    for f in os.listdir(work):
        if f.endswith(".shp") and "county" in f.lower():
            shp_name = os.path.join(work, f)
            break
    sf = shapefile.Reader(shp_name)
    fields = [f[0] for f in sf.fields[1:]]
    features = []
    for sr in sf.shapeRecords():
        props = dict(zip(fields, sr.record))
        geoid   = str(props.get("GEOID","")).zfill(5)
        name    = str(props.get("NAME",""))
        statefp = str(props.get("STATEFP",""))
        geom    = sr.shape.__geo_interface__
        features.append({
            "type":"Feature",
            "properties":{"GEOID":geoid,"NAME":name,"STATEFP":statefp},
            "geometry": geom,
        })
    with open(out, "w") as fh:
        json.dump({"type":"FeatureCollection","features":features}, fh)
    print(f"  wrote {len(features)} counties to {out}")
    sys.exit(0)
except Exception as e:
    print(f"ERROR: could not convert shapefile: {e}", file=sys.stderr)
    sys.exit(1)
PYEOF

WORK_DIR="${WORK_DIR}" TIGER_YEAR="${TIGER_YEAR}" python3 - <<'PYEOF'
import os, sys, json, subprocess

work = os.environ.get("WORK_DIR", "/tmp/tiger_counties_py")
year = os.environ.get("TIGER_YEAR", "2023")
geojson_path = os.path.join(work, "counties.geojson")
db_url = os.environ.get("DATABASE_URL", "postgresql://weatherwms:weatherwms@localhost:5432/weatherwms")

STATE_ABBR = {
    "01":"AL","02":"AK","04":"AZ","05":"AR","06":"CA","08":"CO","09":"CT","10":"DE",
    "11":"DC","12":"FL","13":"GA","15":"HI","16":"ID","17":"IL","18":"IN","19":"IA",
    "20":"KS","21":"KY","22":"LA","23":"ME","24":"MD","25":"MA","26":"MI","27":"MN",
    "28":"MS","29":"MO","30":"MT","31":"NE","32":"NV","33":"NH","34":"NJ","35":"NM",
    "36":"NY","37":"NC","38":"ND","39":"OH","40":"OK","41":"OR","42":"PA","44":"RI",
    "45":"SC","46":"SD","47":"TN","48":"TX","49":"UT","50":"VT","51":"VA","53":"WA",
    "54":"WV","55":"WI","56":"WY","60":"AS","66":"GU","69":"MP","72":"PR","78":"VI",
}

with open(geojson_path) as f:
    fc = json.load(f)

features = fc["features"]
print(f"  {len(features)} counties to load")

# Build a CSV for COPY
import io, csv
buf = io.StringIO()
writer = csv.writer(buf, quoting=csv.QUOTE_MINIMAL)
rows = []
for feat in features:
    p = feat["properties"]
    geoid   = str(p.get("GEOID","")).strip().zfill(5)
    name    = str(p.get("NAME","")).strip()
    sfp     = str(p.get("STATEFP","")).strip().zfill(2)
    abbr    = STATE_ABBR.get(sfp, "")
    geom_geojson = json.dumps(feat["geometry"])
    rows.append((geoid, name, sfp, abbr, geom_geojson))

# Use psql to insert via a temp function
sql_statements = []
# Create the table if needed (schema migration should have done this already)
sql_statements.append("""
CREATE TABLE IF NOT EXISTS tiger_counties (
    geoid CHAR(5) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    state_fips CHAR(2),
    state_abbr CHAR(2),
    geom GEOMETRY(MultiPolygon, 4326) NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tiger_counties_geom ON tiger_counties USING GIST(geom);
CREATE INDEX IF NOT EXISTS idx_tiger_counties_state ON tiger_counties(state_abbr);
""")

# Insert in batches using VALUES
batch_size = 200
inserted = 0
for i in range(0, len(rows), batch_size):
    batch = rows[i:i+batch_size]
    vals = []
    for geoid, name, sfp, abbr, geom in batch:
        # Escape single quotes in name
        name_esc = name.replace("'", "''")
        geom_esc = geom.replace("'", "''")
        abbr_esc = abbr or "NULL"
        if abbr:
            vals.append(f"('{geoid}', '{name_esc}', '{sfp}', '{abbr_esc}', "
                        f"ST_Multi(ST_SetSRID(ST_GeomFromGeoJSON('{geom_esc}'), 4326)))")
        else:
            vals.append(f"('{geoid}', '{name_esc}', '{sfp}', NULL, "
                        f"ST_Multi(ST_SetSRID(ST_GeomFromGeoJSON('{geom_esc}'), 4326)))")
    stmt = ("INSERT INTO tiger_counties (geoid, name, state_fips, state_abbr, geom) VALUES "
            + ",\n".join(vals)
            + " ON CONFLICT (geoid) DO UPDATE SET name=EXCLUDED.name, "
              "state_fips=EXCLUDED.state_fips, state_abbr=EXCLUDED.state_abbr, geom=EXCLUDED.geom;")
    sql_statements.append(stmt)
    inserted += len(batch)
    if i % 1000 == 0:
        sys.stdout.write(f"\r  {i}/{len(rows)} ...")
        sys.stdout.flush()

print(f"\r  Building SQL for {len(rows)} rows...")
combined_sql = "\n".join(sql_statements)

# Write to temp file and execute
sql_file = os.path.join(work, "tiger_insert.sql")
with open(sql_file, "w") as f:
    f.write(combined_sql)

print("  Running psql INSERT...")
result = subprocess.run(
    ["psql", db_url, "-v", "ON_ERROR_STOP=1", "-f", sql_file],
    capture_output=True, text=True
)
if result.returncode != 0:
    print(f"ERROR: {result.stderr[-500:]}", file=sys.stderr)
    sys.exit(1)
print(f"  Loaded {len(rows)} counties into tiger_counties")
PYEOF

echo ""
echo "==> Stamping county_fips on existing storm_events (retroactive spatial join)..."
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -c "
UPDATE storm_events se
SET county_fips = (
    SELECT geoid FROM tiger_counties tc
    WHERE ST_Contains(tc.geom, se.geom_point)
    LIMIT 1
)
WHERE se.geom_point IS NOT NULL AND se.county_fips IS NULL;
SELECT COUNT(*) AS updated FROM storm_events WHERE county_fips IS NOT NULL;
"

echo "==> Refreshing county aggregate..."
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -c "
REFRESH MATERIALIZED VIEW mv_county_event_counts;
SELECT COUNT(*) AS agg_rows FROM mv_county_event_counts;
"

echo ""
echo "==> Done. Row count:"
psql "${DATABASE_URL}" -c "SELECT COUNT(*) AS counties FROM tiger_counties;"
