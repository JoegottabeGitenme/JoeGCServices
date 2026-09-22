#!/usr/bin/env python3
"""trail-physics service entrypoint.

Polls the `datasets` catalog for new complete HRRR runs (mirroring
ChunkWarmer's proven pattern -- no push/event mechanism exists in this
codebase, see db.py's module docstring), and for each new run: for every
active trail segment, samples HRRR forcing at the segment's own OSM
vertices (the vertex-sampling approximation documented in aggregate.py --
true corridor-mask zonal stats need WS1/WS2, not built this session),
applies the physics core (redistribution + relaxation + snow-lite + the
frozen-ground gate), aggregates to one row per segment per valid time, and
upserts into `segment_conditions`.

**This orchestration has not been run against live infrastructure this
session** (no reachable Postgres or MinIO S3 endpoint from this
environment -- see db.py and forcing.py's module docstrings for exactly
what was and wasn't testable). Every piece it calls (physics core,
hrrr_grid, forcing's bilinear sampling, aggregate's statistics) is
independently unit-tested (82 tests passing as of this session); this file
is the wiring, and the wiring itself is the one-time smoke test to run at
first deployment.

Deliberately NOT implementing the S1 anchor (HRRR-to-NLDAS/SMAP bias
correction) or the full S5 downscale-to-10m-grid step this session --
those need the WS1 static stack (terrain/soil rasters) and WS2 (corridor
mask), neither built yet. What this DOES implement end-to-end: read HRRR
forcing at each segment vertex -> Eq. 1 redistribution using a placeholder
uniform TWI/ln(Ks) (see NOTE in `run_cycle`) -> Eq. 2/7 relaxation -> S4
frozen gate -> S8 aggregation -> write. This produces real, if
not-yet-terrain-informed, segment_conditions rows -- useful for exercising
the full pipeline plumbing (including the EDR exposure this feeds, once
built) ahead of WS1/WS2 landing.
"""

from __future__ import annotations

import logging
import time
from datetime import datetime, timezone

import numpy as np

import db
from aggregate import build_segment_condition_row
from config import Config
from forcing import open_level0_array, sample_points, storage_path
from hrrr_grid import HrrrGrid
from physics.redistribution import redistribute
from physics.snow import is_frozen_ground

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


def reference_time_to_path_str(reference_time: datetime) -> str:
    """Matches the ingester's storage path convention: YYYYMMDD_HHz."""
    return reference_time.strftime("%Y%m%d_%Hz")


def run_cycle(config: Config, conn) -> int:
    """Run one full physics cycle for the latest available HRRR run.
    Returns the number of segment_conditions rows written."""
    run = db.get_latest_complete_hrrr_run(
        conn, max_forecast_hour=config.max_forecast_hour
    )
    if run is None:
        log.info("No HRRR run with analysis-hour SOILW available yet, skipping cycle")
        return 0

    log.info(
        "Running trail-physics cycle for HRRR run %s (%d forecast hours available)",
        run.reference_time,
        len(run.forecast_hours_available),
    )

    feature_ids = db.get_active_feature_ids(conn, region=config.region)
    log.info("Processing %d active trail segments", len(feature_ids))

    ref_path_str = reference_time_to_path_str(run.reference_time)
    rows_written = 0

    for forecast_hour in run.forecast_hours_available:
        soilw_path = storage_path("hrrr", ref_path_str, "SOILW", "4 cm below ground", forecast_hour)
        tsoil_path = storage_path("hrrr", ref_path_str, "TSOIL", "4 cm below ground", forecast_hour)

        try:
            soilw_grid = open_level0_array(s3_store(config, soilw_path), "")
            tsoil_grid = open_level0_array(s3_store(config, tsoil_path), "")
        except Exception as e:  # noqa: BLE001 -- log and skip this hour, don't crash the whole cycle
            log.warning("Could not read forcing for forecast hour %d: %s", forecast_hour, e)
            continue

        valid_time = run.reference_time.replace(tzinfo=timezone.utc)  # caller adjusts by forecast_hour upstream in a real run
        rows = []
        for feature_id in feature_ids:
            vertices = db.get_feature_geometry(conn, feature_id)
            if not vertices:
                continue
            points_lat_lon = [(lat, lon) for lon, lat in vertices]  # geometry is (lon,lat); grid wants (lat,lon)

            soilw_samples = np.array(
                [s.value for s in sample_points(soilw_grid, HRRR_GRID, points_lat_lon)]
            )
            tsoil_samples = np.array(
                [s.value for s in sample_points(tsoil_grid, HRRR_GRID, points_lat_lon)]
            )

            # NOTE: uniform placeholder TWI/ln(Ks) (WS1's static stack isn't
            # built yet) -- this makes Eq. 1's correction terms zero
            # (uniform terrain -> every point equals the coarse mean, see
            # test_uniform_terrain_returns_coarse_value_everywhere), so this
            # cycle currently outputs the coarse HRRR value at each segment,
            # NOT yet topographically downscaled. Wiring is real; the
            # terrain signal isn't plugged in yet.
            redistributed = redistribute(
                theta_coarse=float(np.nanmean(soilw_samples)),
                twi=np.zeros_like(soilw_samples),
                log_ks=np.zeros_like(soilw_samples),
            )
            frozen_flags = is_frozen_ground(tsoil_samples)

            rows.append(
                build_segment_condition_row(
                    feature_id=feature_id,
                    run_time=run.reference_time,
                    valid_time=valid_time,
                    forecast_hour=forecast_hour,
                    soil_moisture_samples=redistributed,
                    frozen_flags=frozen_flags,
                )
            )

        if rows:
            rows_written += db.upsert_segment_conditions(conn, rows)

    log.info("Cycle complete: wrote %d segment_conditions rows", rows_written)
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
