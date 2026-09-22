"""Postgres access for the trail-physics service: reads the `datasets`
catalog (written by the Rust ingester) to find HRRR forcing grids, reads
`linear_features` for the segment/corridor list, and writes
`segment_conditions` (this service's own output table).

Schema source of truth is `crates/storage/src/catalog.rs` (`datasets`,
`linear_features`) and `SEGMENT_CONDITIONS_SCHEMA_SQL` (`segment_conditions`)
-- this module does not run migrations itself, the ingester does (see
`Catalog::migrate_segment_conditions`, called from
`services/ingester/src/main.rs`).

**Not integration-tested against a live database this session** -- no
Postgres server was reachable from this environment. The SQL is written
directly against the confirmed schema (column names/types cross-checked
against catalog.rs, not guessed), and the query patterns mirror
`ChunkWarmer`'s already-proven "poll datasets for new complete runs"
approach (`services/wms-api/src/chunk_warming.rs`) rather than inventing a
new one -- but treat a first real run against the actual database as a
smoke test, same posture as forcing.py's MinIO reading.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from datetime import datetime

import psycopg


def connect(database_url: str | None = None):
    """Returns a live psycopg connection. `database_url` falls back to the
    DATABASE_URL env var, matching every other service in this codebase."""
    url = database_url or os.environ["DATABASE_URL"]
    return psycopg.connect(url)


@dataclass
class HrrrRun:
    reference_time: datetime
    forecast_hours_available: list[int]


def get_latest_complete_hrrr_run(
    conn, model: str = "hrrr", param: str = "SOILW", level: str = "4 cm below ground", max_forecast_hour: int = 48
) -> HrrrRun | None:
    """Find the most recent HRRR run where at least one dataset row exists
    for `param`/`level` at forecast hour 0 (i.e. the analysis is in) --
    the trigger condition for starting a physics cycle. Returns all
    forecast hours actually available for that run (which may be a subset
    of [0, max_forecast_hour] if the forecast leg is still landing).

    Mirrors ChunkWarmer's polling pattern (services/wms-api/src/
    chunk_warming.rs) rather than requiring a push/event from the ingester,
    since no such event mechanism exists in this codebase (confirmed during
    the ingredients-fetch session's infra audit).
    """
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT reference_time
            FROM datasets
            WHERE model = %s AND parameter = %s AND level = %s
              AND forecast_hour = 0 AND status = 'available'
            ORDER BY reference_time DESC
            LIMIT 1
            """,
            (model, param, level),
        )
        row = cur.fetchone()
        if row is None:
            return None
        reference_time = row[0]

        cur.execute(
            """
            SELECT forecast_hour
            FROM datasets
            WHERE model = %s AND parameter = %s AND level = %s
              AND reference_time = %s AND forecast_hour <= %s
              AND status = 'available'
            ORDER BY forecast_hour ASC
            """,
            (model, param, level, reference_time, max_forecast_hour),
        )
        hours = [r[0] for r in cur.fetchall()]

    return HrrrRun(reference_time=reference_time, forecast_hours_available=hours)


def get_storage_path(
    conn, model: str, param: str, level: str, reference_time: datetime, forecast_hour: int
) -> str | None:
    """Look up the exact MinIO object path for one grid (avoids
    reconstructing it from the naming convention -- the catalog is the
    source of truth, not a parallel guess)."""
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT storage_path FROM datasets
            WHERE model = %s AND parameter = %s AND level = %s
              AND reference_time = %s AND forecast_hour = %s
              AND status = 'available'
            """,
            (model, param, level, reference_time, forecast_hour),
        )
        row = cur.fetchone()
        return row[0] if row else None


def get_active_feature_ids(conn, region: str | None = None) -> list[int]:
    """Active trail way ids (feature_class column not filtered here --
    aggregate.py's caller decides whether to restrict to mtb_trail only or
    include hiking/track/bridleway too)."""
    with conn.cursor() as cur:
        if region:
            cur.execute(
                "SELECT feature_id FROM linear_features WHERE active = TRUE AND region = %s",
                (region,),
            )
        else:
            cur.execute("SELECT feature_id FROM linear_features WHERE active = TRUE")
        return [r[0] for r in cur.fetchall()]


def get_feature_geometry(conn, feature_id: int) -> list[tuple[float, float]] | None:
    """Ordered (lon, lat) vertices for one trail way, via PostGIS
    ST_AsGeoJSON (same pattern already used by
    crates/storage/src/linear_features.rs's Rust reads -- this is just the
    Python-side equivalent query)."""
    with conn.cursor() as cur:
        cur.execute(
            "SELECT ST_AsGeoJSON(geom) FROM linear_features WHERE feature_id = %s",
            (feature_id,),
        )
        row = cur.fetchone()
        if row is None:
            return None
        import json

        geojson = json.loads(row[0])
        return [tuple(coord) for coord in geojson["coordinates"]]


def upsert_segment_conditions(conn, rows: list[dict]) -> int:
    """Batch upsert into segment_conditions. Each row dict must have keys
    matching the schema: feature_id, run_time, valid_time, forecast_hour,
    soil_moisture, frozen_fraction, frost_depth_m, swe_mm, softness_index,
    confidence, model_version (optional, defaults server-side).

    ON CONFLICT (feature_id, valid_time, model_version) DO UPDATE, matching
    every other upsert in this codebase's pattern (linear_features,
    trail_reports) -- a re-run of the same cycle overwrites rather than
    duplicates.
    """
    count = 0
    with conn.cursor() as cur:
        for row in rows:
            cur.execute(
                """
                INSERT INTO segment_conditions (
                    feature_id, run_time, valid_time, forecast_hour,
                    soil_moisture, frozen_fraction, frost_depth_m, swe_mm,
                    softness_index, confidence, model_version, raw
                ) VALUES (
                    %(feature_id)s, %(run_time)s, %(valid_time)s, %(forecast_hour)s,
                    %(soil_moisture)s, %(frozen_fraction)s, %(frost_depth_m)s, %(swe_mm)s,
                    %(softness_index)s, %(confidence)s,
                    %(model_version)s, %(raw)s
                )
                ON CONFLICT (feature_id, valid_time, model_version) DO UPDATE SET
                    run_time = EXCLUDED.run_time,
                    forecast_hour = EXCLUDED.forecast_hour,
                    soil_moisture = EXCLUDED.soil_moisture,
                    frozen_fraction = EXCLUDED.frozen_fraction,
                    frost_depth_m = EXCLUDED.frost_depth_m,
                    swe_mm = EXCLUDED.swe_mm,
                    softness_index = EXCLUDED.softness_index,
                    confidence = EXCLUDED.confidence,
                    raw = EXCLUDED.raw,
                    ingested_at = NOW()
                """,
                {
                    "model_version": "trail-physics-v0",
                    "raw": "{}",
                    **row,
                },
            )
            count += 1
    conn.commit()
    return count
