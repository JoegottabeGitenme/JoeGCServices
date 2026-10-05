"""Tests for db.py's Session 12 job-ledger functions. No live Postgres is
reachable from this environment (see db.py's own module docstring) --
these tests use a lightweight fake cursor/connection to verify the query
structure, parameter binding, and result parsing without a real database,
the same posture as forcing.py's synthetic-Zarr tests standing in for a
live MinIO read."""

import sys
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import MagicMock

sys.path.insert(0, str(Path(__file__).parent.parent))

import db  # noqa: E402


class FakeCursor:
    def __init__(self, fetchall_result=None):
        self._fetchall_result = fetchall_result or []
        self.executed_sql = None
        self.executed_params = None

    def execute(self, sql, params=None):
        self.executed_sql = sql
        self.executed_params = params

    def fetchall(self):
        return self._fetchall_result

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


class FakeConnection:
    def __init__(self, cursor: FakeCursor):
        self._cursor = cursor
        self.committed = False

    def cursor(self):
        return self._cursor

    def commit(self):
        self.committed = True


class TestGetPendingForecastHours:
    def test_parses_rows_into_pending_forecast_hour_objects(self):
        ref_time = datetime(2026, 1, 15, 12, 0, tzinfo=timezone.utc)
        cursor = FakeCursor(fetchall_result=[(ref_time, 0), (ref_time, 1), (ref_time, 2)])
        conn = FakeConnection(cursor)

        result = db.get_pending_forecast_hours(conn)

        assert len(result) == 3
        assert result[0] == db.PendingForecastHour(reference_time=ref_time, forecast_hour=0)
        assert result[1].forecast_hour == 1
        assert result[2].forecast_hour == 2

    def test_empty_result_returns_empty_list(self):
        cursor = FakeCursor(fetchall_result=[])
        conn = FakeConnection(cursor)
        assert db.get_pending_forecast_hours(conn) == []

    def test_query_requires_both_soilw_and_tsoil_via_self_join(self):
        """The whole point of this query is 'both inputs landed' -- confirm
        the SQL actually joins datasets against itself on the TSOIL
        parameter, not just filtering on SOILW alone (which would trigger
        on SOILW arriving even if TSOIL hadn't yet)."""
        cursor = FakeCursor()
        conn = FakeConnection(cursor)
        db.get_pending_forecast_hours(conn)
        sql = cursor.executed_sql
        assert "JOIN datasets t" in sql
        assert "tsoil_param" in sql
        assert "NOT EXISTS" in sql
        assert "trail_physics_progress" in sql

    def test_passes_through_configured_parameters(self):
        cursor = FakeCursor()
        conn = FakeConnection(cursor)
        db.get_pending_forecast_hours(
            conn, model="hrrr", max_forecast_hour=24, model_version="trail-physics-v2", lookback_hours=48,
        )
        params = cursor.executed_params
        assert params["model"] == "hrrr"
        assert params["max_forecast_hour"] == 24
        assert params["model_version"] == "trail-physics-v2"
        assert params["lookback_hours"] == 48

    def test_ordered_oldest_first(self):
        """So a restart resumes near where it left off rather than
        jumping to the newest data and leaving a gap -- confirmed by
        checking the SQL text asks for ascending order (result parsing
        itself can't prove ordering since FakeCursor just returns
        whatever list it's given)."""
        cursor = FakeCursor()
        conn = FakeConnection(cursor)
        db.get_pending_forecast_hours(conn)
        assert "ORDER BY s.reference_time ASC, s.forecast_hour ASC" in cursor.executed_sql


class TestMarkForecastHourProcessed:
    def test_executes_upsert_and_commits(self):
        cursor = MagicMock()
        cursor.__enter__ = MagicMock(return_value=cursor)
        cursor.__exit__ = MagicMock(return_value=False)
        conn = MagicMock()
        conn.cursor.return_value = cursor

        ref_time = datetime(2026, 1, 15, 12, 0, tzinfo=timezone.utc)
        db.mark_forecast_hour_processed(
            conn, reference_time=ref_time, forecast_hour=3, rows_written=42, model_version="trail-physics-v1",
        )

        cursor.execute.assert_called_once()
        sql, params = cursor.execute.call_args[0]
        assert "INSERT INTO trail_physics_progress" in sql
        assert "ON CONFLICT" in sql
        assert params == ("hrrr", ref_time, 3, "trail-physics-v1", 42)
        conn.commit.assert_called_once()


class TestUpsertSegmentConditionsSaturation:
    """Session 14: the saturation column must be written on insert AND
    refreshed on conflict (a re-run of the same hour overwrites, doesn't
    keep a stale value)."""

    def _run(self, row):
        cursor = MagicMock()
        cursor.__enter__ = MagicMock(return_value=cursor)
        cursor.__exit__ = MagicMock(return_value=False)
        conn = MagicMock()
        conn.cursor.return_value = cursor
        db.upsert_segment_conditions(conn, [row])
        return cursor.execute.call_args[0]

    def _row(self, **extra):
        t = datetime(2026, 1, 15, 12, 0, tzinfo=timezone.utc)
        return dict(
            feature_id=1, run_time=t, valid_time=t, forecast_hour=0, soil_moisture=0.2, frozen_fraction=0.0,
            frost_depth_m=None, swe_mm=None, softness_index=None, confidence=1.0, **extra,
        )

    def test_sql_inserts_and_updates_saturation(self):
        sql, _ = self._run(self._row(saturation=0.5))
        assert "saturation" in sql.split("VALUES")[0]  # column list
        assert "%(saturation)s" in sql
        assert "saturation = EXCLUDED.saturation" in sql

    def test_value_is_passed_through(self):
        _, params = self._run(self._row(saturation=0.5))
        assert params["saturation"] == 0.5

    def test_row_without_saturation_key_defaults_to_null_not_a_keyerror(self):
        _, params = self._run(self._row())
        assert params["saturation"] is None


class TestGetActiveFeatureIdsInBbox:
    """Session 13: the coverage-filter query that keeps trail-physics from
    attempting all ~135,000 active Colorado trails every cycle."""

    def test_uses_st_intersects_with_geography_cast(self):
        """Must be a metrically-correct geography intersection (matching
        pipelines/corridor/build_corridor_mask.py's own ST_Buffer
        precedent), not a raw planar bbox comparison."""
        cursor = FakeCursor(fetchall_result=[(1,), (2,), (3,)])
        conn = FakeConnection(cursor)
        result = db.get_active_feature_ids_in_bbox(conn, -105.6, 39.85, -105.1, 40.15)
        assert result == [1, 2, 3]
        assert "ST_Intersects" in cursor.executed_sql
        assert "::geography" in cursor.executed_sql
        assert "ST_MakeEnvelope" in cursor.executed_sql
        assert "active = TRUE" in cursor.executed_sql

    def test_bbox_values_passed_through(self):
        cursor = FakeCursor()
        conn = FakeConnection(cursor)
        db.get_active_feature_ids_in_bbox(conn, -105.6, 39.85, -105.1, 40.15)
        assert cursor.executed_params == (-105.6, 39.85, -105.1, 40.15)

    def test_region_filter_included_when_given(self):
        cursor = FakeCursor()
        conn = FakeConnection(cursor)
        db.get_active_feature_ids_in_bbox(conn, -105.6, 39.85, -105.1, 40.15, region="colorado")
        assert "region = %s" in cursor.executed_sql
        assert cursor.executed_params == ("colorado", -105.6, 39.85, -105.1, 40.15)

    def test_empty_result(self):
        cursor = FakeCursor(fetchall_result=[])
        conn = FakeConnection(cursor)
        assert db.get_active_feature_ids_in_bbox(conn, -105.6, 39.85, -105.1, 40.15) == []


class TestGetFeatureGeometries:
    def test_empty_input_returns_empty_dict_without_querying(self):
        cursor = FakeCursor()
        conn = FakeConnection(cursor)
        result = db.get_feature_geometries(conn, [])
        assert result == {}
        assert cursor.executed_sql is None  # never even ran a query

    def test_parses_multiple_features_into_dict(self):
        import json

        geojson_1 = json.dumps({"type": "LineString", "coordinates": [[-105.2, 39.75], [-105.21, 39.76]]})
        geojson_2 = json.dumps({"type": "LineString", "coordinates": [[-105.3, 39.80]]})
        cursor = FakeCursor(fetchall_result=[(1, geojson_1), (2, geojson_2)])
        conn = FakeConnection(cursor)

        result = db.get_feature_geometries(conn, [1, 2])

        assert result[1] == [(-105.2, 39.75), (-105.21, 39.76)]
        assert result[2] == [(-105.3, 39.80)]

    def test_uses_any_array_query_not_per_feature_loop(self):
        """The whole point -- one query for the batch, confirmed by
        checking the SQL uses ANY(%s) against the full feature_ids list,
        not N separate calls."""
        cursor = FakeCursor(fetchall_result=[])
        conn = FakeConnection(cursor)
        db.get_feature_geometries(conn, [1, 2, 3])
        assert "ANY(%s)" in cursor.executed_sql
        assert cursor.executed_params == ([1, 2, 3],)
