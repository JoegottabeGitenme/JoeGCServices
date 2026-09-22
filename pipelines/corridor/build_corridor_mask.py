#!/usr/bin/env python3
"""WS2 -- corridor mask: buffer active trail geometry into the zone the
physics pipeline should treat as "on/near a trail" for S8 aggregation.

Scope this session: the **vector buffering step** (buffer each active
`linear_features` way by a configurable distance, e.g. the design doc's
"~15m" S8 buffer or a wider corridor for the statewide-domain decision),
written to a new `trail_corridor` table. This is the primitive both a
raster-rasterize-onto-the-WS1-grid approach and a pure-vector (ST_Contains-
per-cell) approach need -- it doesn't depend on WS1's static-stack grid
definition existing yet, so it's buildable and useful now.

**Not implemented this session**: rasterizing the buffered corridor onto
WS1's actual 10m grid (that grid's exact origin/extent doesn't exist yet --
see pipelines/static/README.md). Once it does, add a
`rasterize_onto_grid(corridor_table, grid_bbox, cellsize)` step here that
produces the cell-index <-> feature_id mapping aggregate.py's true S8 (not
the current vertex-sampling approximation) will consume.

Usage:
    python3 build_corridor_mask.py --buffer-m 15 --database-url postgresql://...
"""

from __future__ import annotations

import argparse
import logging

import psycopg

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("build_corridor_mask")

SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS trail_corridor (
    feature_id BIGINT PRIMARY KEY,
    buffer_m REAL NOT NULL,
    geom GEOMETRY(Polygon, 4326) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_trail_corridor_geom ON trail_corridor USING GIST(geom);
"""

# Buffering in degrees is wrong at scale (a degree of longitude shrinks
# toward the poles) -- cast to geography for a metrically-correct buffer,
# then back to geometry for storage/indexing consistency with
# linear_features.geom. This mirrors how ST_DWithin(geom::geography, ...)
# is already used elsewhere in this codebase (e.g. linear_features'
# get_features_in_radius) for the same reason.
UPSERT_SQL = """
INSERT INTO trail_corridor (feature_id, buffer_m, geom, updated_at)
SELECT
    feature_id,
    %(buffer_m)s,
    ST_Buffer(geom::geography, %(buffer_m)s)::geometry,
    NOW()
FROM linear_features
WHERE active = TRUE
ON CONFLICT (feature_id) DO UPDATE SET
    buffer_m = EXCLUDED.buffer_m,
    geom = EXCLUDED.geom,
    updated_at = NOW()
"""

DEACTIVATE_STALE_SQL = """
DELETE FROM trail_corridor
WHERE feature_id NOT IN (SELECT feature_id FROM linear_features WHERE active = TRUE)
"""


def build_corridor_mask(conn, buffer_m: float) -> int:
    with conn.cursor() as cur:
        for statement in SCHEMA_SQL.split(";"):
            trimmed = statement.strip()
            if trimmed:
                cur.execute(trimmed)

        cur.execute(UPSERT_SQL, {"buffer_m": buffer_m})
        upserted = cur.rowcount

        cur.execute(DEACTIVATE_STALE_SQL)
        removed = cur.rowcount

    conn.commit()
    log.info("Corridor mask: %d features buffered, %d stale corridors removed", upserted, removed)
    return upserted


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--buffer-m", type=float, default=15.0, help="Buffer distance in meters (design doc S8 default: ~15m)")
    parser.add_argument("--database-url", required=True)
    args = parser.parse_args()

    conn = psycopg.connect(args.database_url)
    try:
        build_corridor_mask(conn, args.buffer_m)
    finally:
        conn.close()


if __name__ == "__main__":
    main()
