"""Tests for main.py's Session 12/13 orchestration -- the
valid_time = reference_time + forecast_hour bug fix (previously valid_time
was reference_time alone, causing every forecast hour to collide on
segment_conditions's own UNIQUE(feature_id, valid_time, model_version)
upsert key), the open_static_stack graceful-failure fallback, and Session
13's coverage-filtered (not statewide) processing. No live Postgres/MinIO
reachable from this environment -- these tests mock db.py/forcing.py/
static_stack.py's calls, the same posture as every other
not-integration-tested module in this service.
"""

import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import MagicMock, patch

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

import db  # noqa: E402
import main  # noqa: E402
from config import Config  # noqa: E402


def _fake_config(**overrides):
    defaults = dict(
        database_url="postgresql://fake",
        s3_endpoint="http://fake:9000",
        s3_bucket="fake-bucket",
        s3_access_key="x",
        s3_secret_key="x",
        poll_interval_secs=60,
        max_forecast_hour=48,
        region=None,
        static_stack_path=None,
        model_version="trail-physics-v1",
        lookback_hours=72,
    )
    defaults.update(overrides)
    return Config(**defaults)


def _fake_static_stack(bbox=(-105.6, 39.85, -105.1, 40.15)):
    stack = MagicMock()
    stack.wgs84_bbox.return_value = bbox
    return stack


class TestOpenStaticStack:
    def test_none_path_returns_none(self):
        config = _fake_config(static_stack_path=None)
        assert main.open_static_stack(config) is None

    def test_unreachable_stack_falls_back_to_none_not_exception(self):
        """A missing/unreachable static stack is a real, expected
        situation (nothing uploaded yet, or a region not yet covered) --
        must never crash the cycle."""
        config = _fake_config(static_stack_path="static/colorado-10m/pilot")
        with patch("main.StaticStack", side_effect=Exception("no such path")):
            assert main.open_static_stack(config) is None


class TestProcessForecastHourValidTime:
    def test_valid_time_is_reference_time_plus_forecast_hour(self):
        """THE bug fix: forecast hour 5 must produce a valid_time 5 hours
        after reference_time, not equal to reference_time."""
        config = _fake_config()
        reference_time = datetime(2026, 1, 15, 12, 0)  # naive, like a real psycopg TIMESTAMPTZ read
        job = db.PendingForecastHour(reference_time=reference_time, forecast_hour=5)

        captured_rows = []

        def fake_build_row(**kwargs):
            captured_rows.append(kwargs)
            return kwargs

        with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
             patch("main.sample_points", return_value=[]), \
             patch("main.build_segment_condition_row", side_effect=fake_build_row), \
             patch("main.db") as mock_db, \
             patch("main.is_frozen_ground", return_value=np.array([])), \
             patch("main.downscale_soil_moisture") as mock_downscale:
            mock_db.upsert_segment_conditions.return_value = 1
            mock_downscale.return_value = MagicMock(predicted=np.array([0.2]), confidence=0.0)

            main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1],
                feature_geometries={1: [(-105.2, 39.75)]}, static_stack=None, job=job,
            )

        assert len(captured_rows) == 1
        expected_valid_time = reference_time.replace(tzinfo=timezone.utc) + timedelta(hours=5)
        assert captured_rows[0]["valid_time"] == expected_valid_time

    def test_forecast_hour_zero_valid_time_equals_reference_time(self):
        config = _fake_config()
        reference_time = datetime(2026, 1, 15, 12, 0)
        job = db.PendingForecastHour(reference_time=reference_time, forecast_hour=0)

        captured_rows = []

        def fake_build_row(**kwargs):
            captured_rows.append(kwargs)
            return kwargs

        with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
             patch("main.sample_points", return_value=[]), \
             patch("main.build_segment_condition_row", side_effect=fake_build_row), \
             patch("main.db") as mock_db, \
             patch("main.is_frozen_ground", return_value=np.array([])), \
             patch("main.downscale_soil_moisture") as mock_downscale:
            mock_db.upsert_segment_conditions.return_value = 1
            mock_downscale.return_value = MagicMock(predicted=np.array([0.2]), confidence=0.0)

            main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1],
                feature_geometries={1: [(-105.2, 39.75)]}, static_stack=None, job=job,
            )

        assert captured_rows[0]["valid_time"] == reference_time.replace(tzinfo=timezone.utc)

    def test_marks_forecast_hour_processed_after_upsert(self):
        """The job ledger must be updated so the next poll doesn't
        reprocess this same forecast hour."""
        config = _fake_config()
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=3)

        with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
             patch("main.sample_points", return_value=[]), \
             patch("main.build_segment_condition_row", return_value={}), \
             patch("main.db") as mock_db, \
             patch("main.is_frozen_ground", return_value=np.array([])), \
             patch("main.downscale_soil_moisture") as mock_downscale:
            mock_db.upsert_segment_conditions.return_value = 7
            mock_downscale.return_value = MagicMock(predicted=np.array([0.2]), confidence=1.0)

            main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1],
                feature_geometries={1: [(-105.2, 39.75)]}, static_stack=None, job=job,
            )

            mock_db.mark_forecast_hour_processed.assert_called_once()
            call_kwargs = mock_db.mark_forecast_hour_processed.call_args.kwargs
            assert call_kwargs["reference_time"] == job.reference_time
            assert call_kwargs["forecast_hour"] == 3
            assert call_kwargs["rows_written"] == 7
            assert call_kwargs["model_version"] == "trail-physics-v1"

    def test_unreadable_forcing_skips_hour_without_crashing(self):
        config = _fake_config()
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=3)
        with patch("main.open_level0_array", side_effect=Exception("not found in MinIO")), \
             patch("main.db") as mock_db:
            rows_written = main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1],
                feature_geometries={1: [(-105.2, 39.75)]}, static_stack=None, job=job,
            )
        assert rows_written == 0
        mock_db.mark_forecast_hour_processed.assert_not_called()  # don't mark done -- retry next cycle

    def test_feature_with_no_geometry_is_skipped_not_errored(self):
        """A feature_id present in feature_ids but absent from
        feature_geometries (e.g. deleted between the two queries) must be
        silently skipped, not raise."""
        config = _fake_config()
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)

        captured_rows = []

        def fake_build_row(**kwargs):
            captured_rows.append(kwargs)
            return kwargs

        with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
             patch("main.sample_points", return_value=[]), \
             patch("main.build_segment_condition_row", side_effect=fake_build_row), \
             patch("main.db") as mock_db, \
             patch("main.is_frozen_ground", return_value=np.array([])), \
             patch("main.downscale_soil_moisture") as mock_downscale:
            mock_db.upsert_segment_conditions.return_value = 0
            mock_downscale.return_value = MagicMock(predicted=np.array([0.2]), confidence=0.0)

            main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1, 2],
                feature_geometries={1: [(-105.2, 39.75)]},  # feature 2 missing
                static_stack=None, job=job,
            )

        assert len(captured_rows) == 1  # only feature 1 processed


class TestStaticStackSampledOncePerForecastHour:
    """Session 14: the actual regression test for the live-profiled
    performance bug -- `sample_static_inputs` (the real I/O) must be
    called exactly ONCE per forecast hour, covering every point from
    every processed segment in one batch, never once per segment. The
    old per-segment call pattern was measured live in production to cost
    several minutes per forecast hour (thousands of tiny S3 reads),
    slower than HRRR's own ingest rate -- see downscale.py's own module
    docstring."""

    def test_sample_static_inputs_called_once_regardless_of_feature_count(self):
        config = _fake_config()
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)
        static_stack = _fake_static_stack()

        with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
             patch("main.sample_points", return_value=[MagicMock(value=0.2), MagicMock(value=0.2)]), \
             patch("main.build_segment_condition_row", return_value={}), \
             patch("main.db") as mock_db, \
             patch("main.is_frozen_ground", return_value=np.array([0.0, 0.0])), \
             patch("main.sample_static_inputs") as mock_sample_static, \
             patch("main.downscale_soil_moisture") as mock_downscale:
            mock_db.upsert_segment_conditions.return_value = 5
            fake_samples = MagicMock()
            fake_samples.slice.return_value = MagicMock()
            mock_sample_static.return_value = fake_samples
            mock_downscale.return_value = MagicMock(predicted=np.array([0.2, 0.2]), confidence=1.0)

            main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1, 2, 3, 4, 5],
                feature_geometries={
                    1: [(-105.2, 39.75)], 2: [(-105.2, 39.75)], 3: [(-105.2, 39.75)],
                    4: [(-105.2, 39.75)], 5: [(-105.2, 39.75)],
                },
                static_stack=static_stack, job=job,
            )

        # The whole point: 1 call total, not 5 (one per feature).
        assert mock_sample_static.call_count == 1
        # And downscale_soil_moisture (pure combination, no I/O) is fine
        # to call per-segment -- that's not the expensive part.
        assert mock_downscale.call_count == 5

    def test_no_static_stack_never_calls_sample_static_inputs(self):
        config = _fake_config()
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)

        with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
             patch("main.sample_points", return_value=[MagicMock(value=0.2)]), \
             patch("main.build_segment_condition_row", return_value={}), \
             patch("main.db") as mock_db, \
             patch("main.is_frozen_ground", return_value=np.array([0.0])), \
             patch("main.sample_static_inputs") as mock_sample_static, \
             patch("main.downscale_soil_moisture") as mock_downscale:
            mock_db.upsert_segment_conditions.return_value = 1
            mock_downscale.return_value = MagicMock(predicted=np.array([0.2]), confidence=0.0)

            main._process_forecast_hour(
                config, conn=MagicMock(), feature_ids=[1],
                feature_geometries={1: [(-105.2, 39.75)]}, static_stack=None, job=job,
            )

        mock_sample_static.assert_not_called()
        mock_downscale.assert_called_once()
        # Called with static_samples=None since there's no stack.
        assert mock_downscale.call_args.args[0] is None


class TestRunCycle:
    def test_no_pending_work_returns_zero(self):
        config = _fake_config()
        with patch("main.db") as mock_db:
            mock_db.get_pending_forecast_hours.return_value = []
            result = main.run_cycle(config, conn=MagicMock())
        assert result == 0

    def test_no_static_stack_skips_all_processing(self):
        """Session 13: without a static stack, the cycle must do NO
        segment processing at all (not fall back to statewide raw-HRRR
        processing, which reintroduces the exact sizing problem coverage
        filtering exists to avoid)."""
        config = _fake_config()
        jobs = [db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)]
        with patch("main.db") as mock_db, patch("main.open_static_stack", return_value=None):
            mock_db.get_pending_forecast_hours.return_value = jobs
            result = main.run_cycle(config, conn=MagicMock())
        assert result == 0
        mock_db.get_active_feature_ids_in_bbox.assert_not_called()

    def test_no_features_in_coverage_returns_zero(self):
        config = _fake_config()
        jobs = [db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)]
        stack = _fake_static_stack()
        with patch("main.db") as mock_db, patch("main.open_static_stack", return_value=stack):
            mock_db.get_pending_forecast_hours.return_value = jobs
            mock_db.get_active_feature_ids_in_bbox.return_value = []
            result = main.run_cycle(config, conn=MagicMock())
        assert result == 0

    def test_processes_each_pending_job_within_coverage(self):
        config = _fake_config()
        jobs = [
            db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0),
            db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=1),
        ]
        stack = _fake_static_stack()
        with patch("main.db") as mock_db, patch("main.open_static_stack", return_value=stack), \
             patch("main._process_forecast_hour", return_value=5) as mock_process:
            mock_db.get_pending_forecast_hours.return_value = jobs
            mock_db.get_active_feature_ids_in_bbox.return_value = [1, 2, 3]
            mock_db.get_feature_geometries.return_value = {1: [(-105.2, 39.75)]}
            result = main.run_cycle(config, conn=MagicMock())
        assert result == 10  # 5 + 5
        assert mock_process.call_count == 2

    def test_coverage_query_uses_static_stack_bbox(self):
        """The whole point of Session 13's fix -- confirm the bbox from
        the static stack actually drives the feature-selection query, not
        an unrelated/default region."""
        config = _fake_config()
        jobs = [db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)]
        stack = _fake_static_stack(bbox=(-105.6, 39.85, -105.1, 40.15))
        with patch("main.db") as mock_db, patch("main.open_static_stack", return_value=stack), \
             patch("main._process_forecast_hour", return_value=0):
            mock_db.get_pending_forecast_hours.return_value = jobs
            mock_db.get_active_feature_ids_in_bbox.return_value = [1]
            mock_db.get_feature_geometries.return_value = {1: [(-105.2, 39.75)]}
            main.run_cycle(config, conn=MagicMock())

        mock_db.get_active_feature_ids_in_bbox.assert_called_once()
        call_args = mock_db.get_active_feature_ids_in_bbox.call_args
        assert call_args.args[1:5] == (-105.6, 39.85, -105.1, 40.15)
