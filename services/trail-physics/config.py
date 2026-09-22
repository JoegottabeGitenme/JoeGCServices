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
            poll_interval_secs=int(os.environ.get("TRAIL_PHYSICS_POLL_INTERVAL_SECS", "1800")),
            max_forecast_hour=int(os.environ.get("TRAIL_PHYSICS_MAX_FORECAST_HOUR", "48")),
            region=os.environ.get("TRAIL_PHYSICS_REGION"),  # None = all regions
        )
