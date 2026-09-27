#!/usr/bin/env python3
"""trail-physics service entrypoint.

**Session 12: rewritten as an incremental, ingest-keyed job worker** --
the "automated diagnostic on model ingest" the design doc always intended
this to become, now that the physics is fully validated (Tarrawarra TDR/
NMM, Shale Hills TDR -- Sessions 8-10) and a real static terrain/soil
stack exists (the Boulder-area pilot, Session 11). Polls `datasets` for
(reference_time, forecast_hour) pairs where BOTH SOILW and TSOIL have
landed, that a durable ledger (`trail_physics_progress`) shows haven't
been processed yet under this version -- see db.py's module docstring for
why this replaced the original "reprocess the single latest run every
30 minutes" design (no push/event mechanism exists in this codebase; this
is a much more precise and durable poll, not a push).

For each pending forecast hour, for every active trail segment: samples
HRRR forcing at the segment's own OSM vertices (the vertex-sampling
approximation documented in aggregate.py -- true corridor-mask zonal
stats need WS2, not built yet), downscales via the real Creare/GeoWATCH
production equation using the WS1 static stack where it has coverage
(`downscale.py`, falling back to the raw HRRR value with reduced
confidence elsewhere), applies the S4 frozen-ground gate, aggregates to
one row per segment per valid time, and upserts into `segment_conditions`
-- then marks that forecast hour done in the ledger.

**This orchestration has not been run against live infrastructure this
session** (no reachable Postgres or MinIO S3 endpoint from this
environment -- see db.py/forcing.py/static_stack.py's module docstrings
for exactly what was and wasn't testable). Every piece it calls is
independently unit-tested; this file is the wiring, and the wiring itself
is the one-time smoke test to run at first deployment (see
docker-compose.yml's trail-physics profile-gate comment for exactly
what's still needed before removing that gate).
"""

from __future__ import annotations

import logging
import time
from datetime import timedelta, timezone

import numpy as np

import db
from aggregate import build_segment_condition_row
from config import Config
from downscale import downscale_soil_moisture
from forcing import open_level0_array, sample_points, storage_path
from hrrr_grid import HrrrGrid
from physics.snow import is_frozen_ground
from static_stack import StaticStack

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("trail-physics")

HRRR_GRID = HrrrGrid.hrrr()


def s3_store(config: Config, path: str):
    """Build an s3fs-backed store URL/mapper for zarr.open_group. Kept as
    its own function so tests can substitute a local path instead."""
    import s3fs

    fs = s3fs.S3FileSystem(
        endpoint_url=config.s3_endpoint,
        key=config.s3_access_key,
        secret=config.s3_secret_key,
    )
    return fs.get_mapper(f"{config.s3_bucket}/{path}")


def reference_time_to_path_str(reference_time) -> str:
    """Matches the ingester's storage path convention: YYYYMMDD_HHz."""
    return reference_time.strftime("%Y%m%d_%Hz")


def open_static_stack(config: Config) -> StaticStack | None:
    """Opens the WS1 static stack if `config.static_stack_path` is set and
    reachable. Returns None (not an exception) on any failure to open it
    -- a missing/unreachable static stack is a real, expected situation
    (nothing has been uploaded there yet, or a region config points
    somewhere that hasn't been built), not a reason to crash the whole
    service. As of Session 13, `run_cycle` also uses the stack's own
    coverage envelope to decide WHICH segments to even attempt (see its
    own docstring) -- so a None here means this cycle does no segment
    processing at all, not "process everything with a raw-HRRR fallback"
    (that fallback still exists, in `downscale.py`, for individual points
    that fall just outside the stack's real coverage within an otherwise-
    processed segment -- it is not a substitute for having a stack at
    all)."""
    if not config.static_stack_path:
        return None
    try:
        return StaticStack(s3_store(config, config.static_stack_path))
    except Exception as e:  # noqa: BLE001 -- log and fall back, don't crash the cycle
        log.warning("Could not open static stack at %s: %s -- skipping segment processing this cycle", config.static_stack_path, e)
        return None


def run_cycle(config: Config, conn) -> int:
    """Process every pending (reference_time, forecast_hour) -- one job
    per forecast hour, incrementally, as its inputs land (Session 12).
    Returns the total number of segment_conditions rows written this
    cycle.

    **Session 13: coverage-filtered, not statewide.** Colorado has ~135,000
    active trail segments; HRRR ships ~49 forecast hours per run, hourly --
    processing every active segment for every pending hour is arithmetically
    more work than can complete within one poll interval, long before WS1
    covers more than a small pilot region. Only segments that intersect the
    static stack's own coverage envelope are processed (see
    `db.get_active_feature_ids_in_bbox` / `static_stack.StaticStack.
    wgs84_bbox`) -- everywhere else, the app already reads raw HRRR
    directly (hrrr-soil/hrrr-snow), so a `segment_conditions` row would add
    no information a rider doesn't already have. If no static stack is
    configured/reachable at all, this cycle does no segment processing
    (logged clearly) rather than falling back to "process everything raw" --
    the same sizing problem in a different guise.
    """
    pending = db.get_pending_forecast_hours(
        conn,
        max_forecast_hour=config.max_forecast_hour,
        model_version=config.model_version,
        lookback_hours=config.lookback_hours,
    )
    if not pending:
        log.info("No new complete forecast hours to process")
        return 0

    log.info("Found %d pending forecast hour(s) to process", len(pending))

    static_stack = open_static_stack(config)
    if static_stack is None:
        log.warning(
            "No static stack available (path=%s) -- skipping segment processing this cycle "
            "(processing all active trails statewide would not keep up with hourly HRRR "
            "ingest; see run_cycle's own docstring)",
            config.static_stack_path,
        )
        return 0

    min_lon, min_lat, max_lon, max_lat = static_stack.wgs84_bbox()
    feature_ids = db.get_active_feature_ids_in_bbox(conn, min_lon, min_lat, max_lon, max_lat, region=config.region)
    log.info(
        "Processing against %d active trail segments intersecting the static stack's coverage "
        "(bbox %.4f,%.4f,%.4f,%.4f)",
        len(feature_ids), min_lon, min_lat, max_lon, max_lat,
    )
    if not feature_ids:
        log.info("No active trail segments intersect the static stack's coverage -- nothing to process")
        return 0

    # Fetched once per cycle, not once per feature per forecast hour --
    # trail geometry doesn't change within a cycle (see
    # db.get_feature_geometries's own docstring for why this replaced
    # Session 12's per-forecast-hour-per-feature query).
    feature_geometries = db.get_feature_geometries(conn, feature_ids)

    rows_written_total = 0
    for job in pending:
        rows_written_total += _process_forecast_hour(config, conn, feature_ids, feature_geometries, static_stack, job)

    log.info("Cycle complete: wrote %d segment_conditions rows total", rows_written_total)
    return rows_written_total


def _process_forecast_hour(
    config: Config, conn, feature_ids, feature_geometries: dict, static_stack, job: db.PendingForecastHour
) -> int:
    ref_path_str = reference_time_to_path_str(job.reference_time)
    soilw_path = storage_path("hrrr", ref_path_str, "SOILW", "4 cm below ground", job.forecast_hour)
    tsoil_path = storage_path("hrrr", ref_path_str, "TSOIL", "4 cm below ground", job.forecast_hour)

    try:
        soilw_grid = open_level0_array(s3_store(config, soilw_path), "")
        tsoil_grid = open_level0_array(s3_store(config, tsoil_path), "")
    except Exception as e:  # noqa: BLE001 -- log and skip this hour, don't crash the whole cycle
        log.warning(
            "Could not read forcing for run %s forecast hour %d: %s",
            job.reference_time, job.forecast_hour, e,
        )
        return 0

    # Session 12 bug fix: valid_time MUST be reference_time + forecast_hour,
    # not reference_time alone -- otherwise every forecast hour for a run
    # collides on segment_conditions's own
    # UNIQUE(feature_id, valid_time, model_version) upsert key, and only
    # the last-processed hour's values would ever actually persist.
    reference_time_utc = job.reference_time.replace(tzinfo=timezone.utc)
    valid_time = reference_time_utc + timedelta(hours=job.forecast_hour)

    rows = []
    for feature_id in feature_ids:
        vertices = feature_geometries.get(feature_id)
        if not vertices:
            continue
        points_lat_lon = [(lat, lon) for lon, lat in vertices]  # geometry is (lon,lat); grid wants (lat,lon)

        soilw_samples = np.array([s.value for s in sample_points(soilw_grid, HRRR_GRID, points_lat_lon)])
        tsoil_samples = np.array([s.value for s in sample_points(tsoil_grid, HRRR_GRID, points_lat_lon)])

        result = downscale_soil_moisture(static_stack, HRRR_GRID, points_lat_lon, soilw_samples)
        frozen_flags = is_frozen_ground(tsoil_samples)

        rows.append(
            build_segment_condition_row(
                feature_id=feature_id,
                run_time=reference_time_utc,
                valid_time=valid_time,
                forecast_hour=job.forecast_hour,
                soil_moisture_samples=result.predicted,
                frozen_flags=frozen_flags,
                confidence=result.confidence,
                model_version=config.model_version,
            )
        )

    rows_written = db.upsert_segment_conditions(conn, rows) if rows else 0
    db.mark_forecast_hour_processed(
        conn,
        reference_time=job.reference_time,
        forecast_hour=job.forecast_hour,
        rows_written=rows_written,
        model_version=config.model_version,
    )
    log.info(
        "Run %s forecast hour %d: wrote %d segment_conditions rows",
        job.reference_time, job.forecast_hour, rows_written,
    )
    return rows_written


def run_forever(config: Config) -> None:
    while True:
        try:
            conn = db.connect(config.database_url)
            try:
                run_cycle(config, conn)
            finally:
                conn.close()
        except Exception:  # noqa: BLE001 -- a bad cycle shouldn't kill the service
            log.exception("trail-physics cycle failed")
        time.sleep(config.poll_interval_secs)


if __name__ == "__main__":
    run_forever(Config.from_env())
