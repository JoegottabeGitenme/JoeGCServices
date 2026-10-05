"""Postgres access for the trail-physics service: reads the `datasets`
catalog (written by the Rust ingester) to find HRRR forcing grids, reads
`linear_features` for the segment/corridor list, writes
`segment_conditions` (this service's own output table), and tracks its own
progress in `trail_physics_progress` (also this service's own table).

Schema source of truth is `crates/storage/src/catalog.rs` (`datasets`,
`linear_features`, `SEGMENT_CONDITIONS_SCHEMA_SQL`,
`TRAIL_PHYSICS_PROGRESS_SCHEMA_SQL`) -- this module does not run
migrations itself, the ingester does (`Catalog::migrate_segment_
conditions`/`migrate_trail_physics_progress`, called from
`services/ingester/src/main.rs`).

**Session 12: incremental, ingest-keyed job queue, replacing the original
"reprocess the single latest run on every poll" design.** The trigger is
now "which (reference_time, forecast_hour) pairs have BOTH SOILW and
TSOIL rows available, that this model_version hasn't already processed" --
directly expressing "once these two inputs are both ingested for a given
timestep, run the diagnostic" as a SQL query against the ingester's own
catalog, with a durable ledger (`trail_physics_progress`) recording what's
already been done so a restart doesn't have to redo everything, and so
forecast hours are picked up one at a time as they land rather than
waiting for (or repeatedly reprocessing) a whole run. Still no
push/event mechanism from the ingester (confirmed absent, see this
module's prior docstring and docs/trail-conditions-design.md's Session 12
section) -- this is a poll, just a much more precise and durable one, at
a shorter interval (`config.poll_interval_secs`, now default 60s, not
1800s).

**Not integration-tested against a live database this session** -- no
Postgres server was reachable from this environment. The SQL is written
directly against the confirmed schema (column names/types cross-checked
against catalog.rs, not guessed) -- treat a first real run against the
actual database as a smoke test, same posture as forcing.py's MinIO
reading.
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
class PendingForecastHour:
    reference_time: datetime
    forecast_hour: int


def get_pending_forecast_hours(
    conn,
    model: str = "hrrr",
    soilw_param: str = "SOILW",
    tsoil_param: str = "TSOIL",
    level: str = "4 cm below ground",
    max_forecast_hour: int = 48,
    model_version: str = "trail-physics-v0",
    lookback_hours: int = 72,
) -> list[PendingForecastHour]:
    """The actual trigger condition: every (reference_time, forecast_hour)
    pair where BOTH `soilw_param` and `tsoil_param` have an
    `status='available'` row in `datasets` (i.e. this timestep's two
    physics inputs have both landed), that `trail_physics_progress`
    doesn't already have a row for under this exact `model_version`.

    `lookback_hours` bounds the query to recent runs (default 72h, several
    times HRRR's own forecast range) so this doesn't re-scan the entire
    catalog history every cycle once the ledger is mostly caught up --
    after the very first run against a real database, nearly everything
    within this window will already be in the ledger and get filtered out
    by the NOT EXISTS clause; this is a cheap, bounded query in steady
    state, not an unbounded historical scan.

    Ordered oldest-first so an interrupted cycle (service restart mid-run)
    resumes roughly where it left off rather than jumping to the newest
    data and leaving a gap.
    """
    with conn.cursor() as cur:
        cur.execute(
            """
            SELECT s.reference_time, s.forecast_hour
            FROM datasets s
            JOIN datasets t
              ON t.model = s.model
             AND t.reference_time = s.reference_time
             AND t.forecast_hour = s.forecast_hour
             AND t.parameter = %(tsoil_param)s
             AND t.level = %(level)s
             AND t.status = 'available'
            WHERE s.model = %(model)s
              AND s.parameter = %(soilw_param)s
              AND s.level = %(level)s
              AND s.status = 'available'
              AND s.forecast_hour <= %(max_forecast_hour)s
              AND s.reference_time >= NOW() - (%(lookback_hours)s || ' hours')::interval
              AND NOT EXISTS (
                  SELECT 1 FROM trail_physics_progress p
                  WHERE p.model = %(model)s
                    AND p.reference_time = s.reference_time
                    AND p.forecast_hour = s.forecast_hour
                    AND p.model_version = %(model_version)s
              )
            ORDER BY s.reference_time ASC, s.forecast_hour ASC
            """,
            {
                "model": model,
                "soilw_param": soilw_param,
                "tsoil_param": tsoil_param,
                "level": level,
                "max_forecast_hour": max_forecast_hour,
                "lookback_hours": lookback_hours,
                "model_version": model_version,
            },
        )
        return [PendingForecastHour(reference_time=r[0], forecast_hour=r[1]) for r in cur.fetchall()]


def mark_forecast_hour_processed(
    conn,
    reference_time: datetime,
    forecast_hour: int,
    rows_written: int,
    model: str = "hrrr",
    model_version: str = "trail-physics-v0",
) -> None:
    """Record that this (model, reference_time, forecast_hour) has been
    turned into segment_conditions rows under this model_version -- so
    `get_pending_forecast_hours` won't return it again. Upserts (not a
    plain insert) so re-running the same hour deliberately (e.g. a manual
    backfill after fixing a bug) updates rather than duplicate-erroring."""
    with conn.cursor() as cur:
        cur.execute(
            """
            INSERT INTO trail_physics_progress
                (model, reference_time, forecast_hour, model_version, rows_written)
            VALUES (%s, %s, %s, %s, %s)
            ON CONFLICT (model, reference_time, forecast_hour, model_version) DO UPDATE SET
                rows_written = EXCLUDED.rows_written,
                processed_at = NOW()
            """,
            (model, reference_time, forecast_hour, model_version, rows_written),
        )
    conn.commit()


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


def get_active_feature_ids_in_bbox(
    conn, min_lon: float, min_lat: float, max_lon: float, max_lat: float, region: str | None = None
) -> list[int]:
    """Session 13: active trail way ids that intersect a bounding box --
    used to restrict processing to the WS1 static stack's own coverage
    envelope (`static_stack.StaticStack.wgs84_bbox`) rather than every
    active trail statewide.

    **Why this exists**: `get_active_feature_ids` alone processes all
    active trails (Colorado has ~135,000) for every pending forecast hour
    (HRRR ships ~49 per run, hourly runs) -- arithmetically more work than
    can complete within one poll interval, long before WS1 covers more than
    a small pilot region. Restricting to trails that actually intersect the
    stack's own extent keeps the per-cycle workload proportional to real
    coverage (a few thousand segments for the Boulder-area pilot, not
    135,000) and grows automatically as WS1's coverage grows -- no code
    change needed when a bigger stack replaces the pilot.

    `ST_Intersects` against a lon/lat envelope on a geography-cast geometry
    (matching the metrically-correct pattern already used by
    `pipelines/corridor/build_corridor_mask.py`'s own `ST_Buffer` call, not
    a raw planar bbox test) -- exact at the pilot's small scale, and correct
    at any scale.
    """
    with conn.cursor() as cur:
        envelope_sql = "ST_MakeEnvelope(%s, %s, %s, %s, 4326)"
        if region:
            cur.execute(
                f"""
                SELECT feature_id FROM linear_features
                WHERE active = TRUE AND region = %s
                  AND ST_Intersects(geom::geography, {envelope_sql}::geography)
                """,
                (region, min_lon, min_lat, max_lon, max_lat),
            )
        else:
            cur.execute(
                f"""
                SELECT feature_id FROM linear_features
                WHERE active = TRUE
                  AND ST_Intersects(geom::geography, {envelope_sql}::geography)
                """,
                (min_lon, min_lat, max_lon, max_lat),
            )
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


def get_feature_geometries(conn, feature_ids: list[int]) -> dict[int, list[tuple[float, float]]]:
    """Batch version of `get_feature_geometry` -- one query for the whole
    feature_ids list, rather than one query per feature.

    **Why this exists**: trail geometry doesn't change within a single
    poll cycle (or across a run's forecast hours) -- Session 13's original
    `main.py` called `get_feature_geometry` once per feature PER forecast
    hour, meaning a run with 10 pending hours re-fetched the exact same
    geometry 10 times for every feature. Fetching once per cycle (in
    `run_cycle`, before the forecast-hour loop) and passing the resulting
    dict into `_process_forecast_hour` removes that redundancy -- purely a
    query-count optimization, identical results either way, not a behavior
    change."""
    if not feature_ids:
        return {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT feature_id, ST_AsGeoJSON(geom) FROM linear_features WHERE feature_id = ANY(%s)",
            (feature_ids,),
        )
        import json

        result = {}
        for feature_id, geojson_str in cur.fetchall():
            geojson = json.loads(geojson_str)
            result[feature_id] = [tuple(coord) for coord in geojson["coordinates"]]
        return result


def upsert_segment_conditions(conn, rows: list[dict]) -> int:
    """Batch upsert into segment_conditions. Each row dict must have keys
    matching the schema: feature_id, run_time, valid_time, forecast_hour,
    soil_moisture, saturation, frozen_fraction, frost_depth_m, swe_mm,
    softness_index, confidence, model_version (optional, defaults server-side).

    ON CONFLICT (feature_id, valid_time, model_version) DO UPDATE, matching
    every other upsert in this codebase's pattern (linear_features,
    trail_reports) -- a re-run of the same cycle overwrites rather than
    duplicates.

    **Only an equal-or-newer model run may overwrite** (`WHERE
    segment_conditions.run_time <= EXCLUDED.run_time`, Session 14). Several
    runs predict the same valid hour (run 18z's fh 3 and run 19z's fh 2 are
    the same clock time), and the unique key means they share one row --
    the freshest forecast should win. Without the guard, whichever run is
    *processed last* wins, and a late-arriving hour of an OLDER run (HRRR
    hours trickle in over a run's ingest) would silently replace a fresher
    run's value with a staler one. Equal run_time still updates, so
    re-processing the same run stays idempotent.
    """
    count = 0
    with conn.cursor() as cur:
        for row in rows:
            cur.execute(
                """
                INSERT INTO segment_conditions (
                    feature_id, run_time, valid_time, forecast_hour,
                    soil_moisture, saturation, frozen_fraction, frost_depth_m, swe_mm,
                    softness_index, confidence, model_version, raw
                ) VALUES (
                    %(feature_id)s, %(run_time)s, %(valid_time)s, %(forecast_hour)s,
                    %(soil_moisture)s, %(saturation)s, %(frozen_fraction)s, %(frost_depth_m)s, %(swe_mm)s,
                    %(softness_index)s, %(confidence)s,
                    %(model_version)s, %(raw)s
                )
                ON CONFLICT (feature_id, valid_time, model_version) DO UPDATE SET
                    run_time = EXCLUDED.run_time,
                    forecast_hour = EXCLUDED.forecast_hour,
                    soil_moisture = EXCLUDED.soil_moisture,
                    saturation = EXCLUDED.saturation,
                    frozen_fraction = EXCLUDED.frozen_fraction,
                    frost_depth_m = EXCLUDED.frost_depth_m,
                    swe_mm = EXCLUDED.swe_mm,
                    softness_index = EXCLUDED.softness_index,
                    confidence = EXCLUDED.confidence,
                    raw = EXCLUDED.raw,
                    ingested_at = NOW()
                WHERE segment_conditions.run_time <= EXCLUDED.run_time
                """,
                {
                    "model_version": "trail-physics-v0",
                    "raw": "{}",
                    "saturation": None,  # rows built before Session 14 (or by callers that don't compute it)
                    **row,
                },
            )
            count += 1
    conn.commit()
    return count
