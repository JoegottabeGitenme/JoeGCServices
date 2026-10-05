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
from downscale import StaticSamples  # noqa: E402


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


def _batch(feature_geometries, static_samples=None, feature_ids=None):
    """A real SegmentBatch built by main.build_segment_batch (no static
    stack -> static_samples None) -- exercising the real flattening/offset
    logic, not a mock of it."""
    ids = feature_ids if feature_ids is not None else list(feature_geometries)
    batch = main.build_segment_batch(ids, feature_geometries, static_stack=None)
    if static_samples is not None:
        batch.static_samples = static_samples
    return batch


GOLDEN = [(-105.2, 39.75)]  # one (lon, lat) vertex


def _run_hour(job, batch, config=None, build_row=None, downscale_result=None, upsert_returns=1):
    """Run _process_forecast_hour with I/O mocked (no live MinIO/Postgres
    here -- same posture as every other test of this service) and return
    (captured build_segment_condition_row kwargs, mock_db)."""
    config = config or _fake_config()
    captured = []

    def fake_build_row(**kwargs):
        captured.append(kwargs)
        return kwargs

    with patch("main.open_level0_array", return_value=np.zeros((10, 10))), \
         patch("main.build_segment_condition_row", side_effect=build_row or fake_build_row), \
         patch("main.db") as mock_db, \
         patch("main.is_frozen_ground", side_effect=lambda t: np.zeros(len(t))), \
         patch("main.downscale_soil_moisture") as mock_downscale:
        mock_db.upsert_segment_conditions.return_value = upsert_returns
        mock_downscale.return_value = downscale_result or MagicMock(predicted=np.array([0.2]), confidence=0.0)
        result = main._process_forecast_hour(config, MagicMock(), batch, job)
    return captured, mock_db, mock_downscale, result


class TestProcessForecastHourValidTime:
    def test_valid_time_is_reference_time_plus_forecast_hour(self):
        """THE Session 12 bug fix: forecast hour 5 must produce a valid_time
        5 hours after reference_time, not equal to reference_time."""
        reference_time = datetime(2026, 1, 15, 12, 0)  # naive, like a real psycopg TIMESTAMPTZ read
        job = db.PendingForecastHour(reference_time=reference_time, forecast_hour=5)
        captured, *_ = _run_hour(job, _batch({1: GOLDEN}))
        assert len(captured) == 1
        assert captured[0]["valid_time"] == reference_time.replace(tzinfo=timezone.utc) + timedelta(hours=5)

    def test_forecast_hour_zero_valid_time_equals_reference_time(self):
        reference_time = datetime(2026, 1, 15, 12, 0)
        job = db.PendingForecastHour(reference_time=reference_time, forecast_hour=0)
        captured, *_ = _run_hour(job, _batch({1: GOLDEN}))
        assert captured[0]["valid_time"] == reference_time.replace(tzinfo=timezone.utc)

    def test_marks_forecast_hour_processed_after_upsert(self):
        """The job ledger must be updated so the next poll doesn't
        reprocess this same forecast hour."""
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=3)
        _, mock_db, _, _ = _run_hour(job, _batch({1: GOLDEN}), upsert_returns=7)
        mock_db.mark_forecast_hour_processed.assert_called_once()
        kw = mock_db.mark_forecast_hour_processed.call_args.kwargs
        assert kw["reference_time"] == job.reference_time
        assert kw["forecast_hour"] == 3
        assert kw["rows_written"] == 7
        assert kw["model_version"] == "trail-physics-v1"

    def test_unreadable_forcing_skips_hour_without_crashing(self):
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=3)
        with patch("main.open_level0_array", side_effect=Exception("not found in MinIO")), patch("main.db") as mock_db:
            rows_written = main._process_forecast_hour(_fake_config(), MagicMock(), _batch({1: GOLDEN}), job)
        assert rows_written == 0
        mock_db.mark_forecast_hour_processed.assert_not_called()  # don't mark done -- retry next cycle

    def test_feature_with_no_geometry_is_skipped_not_errored(self):
        """A feature_id present in feature_ids but absent from
        feature_geometries (e.g. deleted between the two queries) must be
        silently skipped, not raise."""
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)
        batch = _batch({1: GOLDEN}, feature_ids=[1, 2])  # feature 2 has no geometry
        captured, *_ = _run_hour(job, batch)
        assert [c["feature_id"] for c in captured] == [1]

    def test_empty_batch_still_marks_hour_done_so_it_isnt_retried_forever(self):
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)
        captured, mock_db, _, result = _run_hour(job, _batch({}, feature_ids=[]))
        assert captured == [] and result == 0
        mock_db.mark_forecast_hour_processed.assert_called_once()
        mock_db.upsert_segment_conditions.assert_not_called()


class TestBuildSegmentBatch:
    def test_offsets_partition_the_flat_arrays_in_feature_order(self):
        geoms = {10: [(-105.2, 39.75), (-105.21, 39.76), (-105.22, 39.77)], 20: [(-105.3, 39.8)], 30: [(-105.4, 39.9), (-105.41, 39.91)]}
        batch = main.build_segment_batch([10, 20, 30], geoms, static_stack=None)
        assert batch.feature_order == [10, 20, 30]
        assert batch.offsets == {10: (0, 3), 20: (3, 4), 30: (4, 6)}
        assert batch.n_points == 6
        assert len(batch.hrrr_rows) == len(batch.hrrr_cols) == 6
        assert batch.static_samples is None

    def test_hrrr_indices_match_the_scalar_projection(self):
        """Rows/cols must be (j, i) from the Rust-validated projection --
        a swapped pair here would sample the wrong HRRR cells silently."""
        batch = main.build_segment_batch([1], {1: [(-105.2211, 39.7555)]}, static_stack=None)
        i, j = main.HRRR_GRID.geo_to_grid(39.7555, -105.2211)
        assert batch.hrrr_rows[0] == pytest.approx(j, abs=1e-7)
        assert batch.hrrr_cols[0] == pytest.approx(i, abs=1e-7)

    def test_features_without_geometry_are_dropped(self):
        batch = main.build_segment_batch([1, 2, 3], {1: [(-105.2, 39.75)], 3: []}, static_stack=None)
        assert batch.feature_order == [1]

    def test_no_geometries_gives_empty_batch(self):
        batch = main.build_segment_batch([1], {}, static_stack=None)
        assert batch.feature_order == [] and batch.n_points == 0 and batch.static_samples is None

    def test_static_stack_sampled_exactly_once_for_all_vertices(self):
        geoms = {i: [(-105.2 - i * 0.001, 39.75), (-105.2 - i * 0.001, 39.76)] for i in range(25)}
        with patch("main.sample_static_inputs") as mock_sample:
            mock_sample.return_value = MagicMock()
            main.build_segment_batch(list(geoms), geoms, static_stack=MagicMock())
        assert mock_sample.call_count == 1
        assert len(mock_sample.call_args.args[1]) == 50  # lats: all 25 x 2 vertices in one call


class TestStaticStackSampledOncePerCycleNotPerHour:
    """Session 14: the static terrain/soil values are time-invariant, so
    they are sampled ONCE per cycle (in build_segment_batch) and reused for
    every pending forecast hour. The Front Range stack is ~20x the pilot;
    re-reading it per hour was the next scaling wall after the per-segment
    pattern fixed earlier this session."""

    def test_run_cycle_samples_static_once_for_many_pending_hours(self):
        config = _fake_config()
        jobs = [db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=h) for h in range(6)]
        stack = _fake_static_stack()
        geoms = {1: [(-105.2, 39.75), (-105.21, 39.76)], 2: [(-105.3, 39.8)]}
        with patch("main.db") as mock_db, patch("main.open_static_stack", return_value=stack), \
             patch("main.sample_static_inputs") as mock_sample, \
             patch("main._process_forecast_hour", return_value=2):
            mock_db.get_pending_forecast_hours.return_value = jobs
            mock_db.get_active_feature_ids_in_bbox.return_value = [1, 2]
            mock_db.get_feature_geometries.return_value = geoms
            mock_sample.return_value = MagicMock()
            main.run_cycle(config, conn=MagicMock())
        assert mock_sample.call_count == 1  # not 6

    def test_every_hour_receives_the_same_batch_object(self):
        config = _fake_config()
        jobs = [db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=h) for h in range(3)]
        with patch("main.db") as mock_db, patch("main.open_static_stack", return_value=_fake_static_stack()), \
             patch("main.sample_static_inputs", return_value=MagicMock()), \
             patch("main._process_forecast_hour", return_value=1) as mock_process:
            mock_db.get_pending_forecast_hours.return_value = jobs
            mock_db.get_active_feature_ids_in_bbox.return_value = [1]
            mock_db.get_feature_geometries.return_value = {1: [(-105.2, 39.75)]}
            main.run_cycle(config, conn=MagicMock())
        batches = [call.args[2] for call in mock_process.call_args_list]
        assert len(batches) == 3 and all(b is batches[0] for b in batches)

    def test_per_hour_processing_never_touches_the_static_stack(self):
        """_process_forecast_hour only gets a batch -- it has no handle on
        the stack at all, so it structurally cannot re-sample it. This
        asserts the sampler isn't reachable from the per-hour path."""
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=1)
        with patch("main.sample_static_inputs") as mock_sample:
            _run_hour(job, _batch({1: GOLDEN}))
        mock_sample.assert_not_called()

    def test_saturation_from_downscale_reaches_the_row_builder(self):
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)
        sat = np.array([0.3])
        result = MagicMock(predicted=np.array([0.2]), confidence=1.0, saturation=sat)
        captured, *_ = _run_hour(job, _batch({1: GOLDEN}), downscale_result=result)
        assert captured[0]["saturation_samples"] is sat

    def test_segment_static_samples_are_sliced_per_feature(self):
        """Each segment must be downscaled against ITS OWN slice of the
        batch's static samples (offsets must line up) -- a misaligned slice
        would apply one trail's terrain to another's soil moisture."""
        samples = StaticSamples(
            twi=np.array([1.0, 2.0, 3.0]), theta_s=np.array([0.4, 0.5, 0.6]),
            theta_wilt=np.array([0.1, 0.1, 0.1]), twi_bar=np.array([8.0, 8.0, 8.0]),
        )
        batch = _batch({1: [(-105.2, 39.75), (-105.21, 39.76)], 2: [(-105.3, 39.8)]}, static_samples=samples)
        job = db.PendingForecastHour(reference_time=datetime(2026, 1, 15, 12, 0), forecast_hour=0)
        *_, mock_downscale, _ = _run_hour(job, batch)
        seg1_static, seg2_static = (c.args[0] for c in mock_downscale.call_args_list)
        np.testing.assert_array_equal(seg1_static.twi, [1.0, 2.0])
        np.testing.assert_array_equal(seg2_static.twi, [3.0])


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
             patch("main.build_segment_batch", return_value=MagicMock(feature_order=[1], n_points=1)), \
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
             patch("main.build_segment_batch", return_value=MagicMock(feature_order=[1], n_points=1)), \
             patch("main._process_forecast_hour", return_value=0):
            mock_db.get_pending_forecast_hours.return_value = jobs
            mock_db.get_active_feature_ids_in_bbox.return_value = [1]
            mock_db.get_feature_geometries.return_value = {1: [(-105.2, 39.75)]}
            main.run_cycle(config, conn=MagicMock())

        mock_db.get_active_feature_ids_in_bbox.assert_called_once()
        call_args = mock_db.get_active_feature_ids_in_bbox.call_args
        assert call_args.args[1:5] == (-105.6, 39.85, -105.1, 40.15)
