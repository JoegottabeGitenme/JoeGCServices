"""Configuration for the trail-physics service, read from environment
variables -- matching every other service in this codebase's convention
(DATABASE_URL, S3_* for MinIO, CONFIG_DIR)."""

from __future__ import annotations

import os
from dataclasses import dataclass


@dataclass
class Config:
    database_url: str
    s3_endpoint: str
    s3_bucket: str
    s3_access_key: str
    s3_secret_key: str
    poll_interval_secs: int
    max_forecast_hour: int
    region: str | None
    static_stack_path: str | None
    model_version: str
    lookback_hours: int

    @classmethod
    def from_env(cls) -> "Config":
        return cls(
            database_url=os.environ.get(
                "DATABASE_URL",
                "postgresql://weatherwms:weatherwms@localhost:5432/weatherwms",
            ),
            s3_endpoint=os.environ.get("S3_ENDPOINT", "http://minio:9000"),
            s3_bucket=os.environ.get("S3_BUCKET", "weather-data"),
            s3_access_key=os.environ.get("S3_ACCESS_KEY", "minioadmin"),
            s3_secret_key=os.environ.get("S3_SECRET_KEY", "minioadmin"),
            # Session 12: down from 1800s -- the trigger is now an
            # incremental, per-forecast-hour job ledger (db.py::
            # get_pending_forecast_hours), not "reprocess the single
            # latest run," so a short poll interval means each forecast
            # hour is picked up within about a minute of landing, matching
            # ChunkWarmer's own default cadence (services/wms-api/src/
            # chunk_warming.rs) rather than inventing a different one.
            poll_interval_secs=int(os.environ.get("TRAIL_PHYSICS_POLL_INTERVAL_SECS", "60")),
            max_forecast_hour=int(os.environ.get("TRAIL_PHYSICS_MAX_FORECAST_HOUR", "48")),
            region=os.environ.get("TRAIL_PHYSICS_REGION"),  # None = all regions
            # WS1 static stack's MinIO object path (Session 11's pilot:
            # "static/colorado-10m/pilot"). None = no static stack
            # available -- every segment falls back to raw HRRR with
            # confidence=0.0 (see downscale.py), which is the correct,
            # honest behavior before WS1 covers a given area, not an error
            # state to avoid running the service in.
            static_stack_path=os.environ.get("TRAIL_PHYSICS_STATIC_STACK_PATH", "static/colorado-10m/pilot"),
            model_version=os.environ.get("TRAIL_PHYSICS_MODEL_VERSION", "trail-physics-v1"),
            lookback_hours=int(os.environ.get("TRAIL_PHYSICS_LOOKBACK_HOURS", "72")),
        )
