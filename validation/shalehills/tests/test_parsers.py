"""Parser tests for the real Shale Hills data (Session 10). Real-data
tests skip gracefully if the data isn't present, matching the pattern
used throughout validation/tarrawarra/tests/.
"""

import datetime
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent.parent))

from parsers import (  # noqa: E402
    PLAUSIBLE_PRESSURE_KPA,
    _parse_date_column_header,
    assign_mukey_to_points,
    dominant_component_texture_by_mukey,
    parse_dem_geotiff,
    parse_meteo_file,
    parse_ssurgo_chorizon,
    parse_ssurgo_component,
    parse_ssurgo_mapunit,
    parse_ssurgo_mapunit_polygons,
    parse_tdr_xlsx,
)

DATA_DIR = Path(__file__).parent.parent / "data"


def _skip_if_absent(*paths: Path):
    for p in paths:
        if not p.exists():
            pytest.skip(f"real data {p} not present (see README.md)")


class TestDateColumnHeaderParsing:
    def test_five_digit_single_digit_month(self):
        """'SM42510' -> 4-25-10 (April 25 2010)."""
        assert _parse_date_column_header("SM42510") == datetime.date(2010, 4, 25)

    def test_six_digit_two_digit_month(self):
        """'SM101010' -> 10-10-10 (October 10 2010), not misread as some
        other split -- the 6-digit form always means a 2-digit month."""
        assert _parse_date_column_header("SM101010") == datetime.date(2010, 10, 10)

    def test_matches_readme_stated_date_range_exactly(self):
        """The real file's earliest/latest column headers must match the
        HydroShare ReadMe's own stated 'Date Start 2006-12-10' / 'Date End
        2015-07-16' EXACTLY -- the strongest possible confirmation this
        parsing rule is correct, not just plausible."""
        assert _parse_date_column_header("SM121006") == datetime.date(2006, 12, 10)
        assert _parse_date_column_header("SM71615") == datetime.date(2015, 7, 16)

    def test_rejects_unrecognized_header(self):
        with pytest.raises(ValueError, match="Unrecognized"):
            _parse_date_column_header("NOTADATE")


class TestParseTdrXlsx:
    def test_real_data_76_dates(self):
        _skip_if_absent(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx")
        readings = parse_tdr_xlsx(str(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=10)
        dates = {r.date for r in readings}
        # Per geowatch.pdf Section 4.2.2 ("76 dates") and confirmed
        # directly against the real file (Session 10).
        assert len(dates) == 76
        assert min(dates) == datetime.date(2006, 12, 10)
        assert max(dates) == datetime.date(2015, 7, 16)

    def test_real_data_values_already_fractional_not_percent(self):
        """Unlike Tarrawarra's %V/V TDR files, this dataset's own values
        are ALREADY m3/m3 -- must never be divided by 100 again."""
        _skip_if_absent(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx")
        readings = parse_tdr_xlsx(str(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=10)
        values = [r.moisture_frac for r in readings]
        # A real %V/V value accidentally left un-divided would show up as
        # mostly 15-45 (typical volumetric moisture percentages); real
        # fractional values must be well under 1.0. (The 8 real literal-0.0
        # data-quality artifacts are already excluded by parse_tdr_xlsx --
        # see its docstring -- so > 0.0 is a real, meaningful assertion
        # here, not just documentation of a known gap.)
        assert all(0.0 < v < 1.0 for v in values)
        assert np.mean(values) < 0.6

    def test_real_data_excludes_literal_zero_readings(self):
        """The 8 real 0.0 data-quality artifacts (Session 10 -- see
        parse_tdr_xlsx's docstring) must never appear in the output."""
        _skip_if_absent(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx")
        readings = parse_tdr_xlsx(str(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=10)
        assert not any(r.moisture_frac == 0.0 for r in readings)

    def test_real_data_coordinates_are_nad83_utm18n_not_state_plane(self):
        """Decisive check (see module docstring): the site coordinate
        cloud's bounding box must match HydroShare's own independently
        stated WGS84 bbox when interpreted as NAD83 UTM Zone 18N."""
        _skip_if_absent(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx")
        from pyproj import Transformer

        readings = parse_tdr_xlsx(str(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=10)
        xs = [r.x for r in readings]
        ys = [r.y for r in readings]
        t = Transformer.from_crs("EPSG:26918", "EPSG:4326", always_xy=True)
        lons, lats = [], []
        for x, y in zip(xs, ys):
            lon, lat = t.transform(x, y)
            lons.append(lon)
            lats.append(lat)
        assert min(lons) == pytest.approx(-77.9071, abs=0.001)
        assert max(lons) == pytest.approx(-77.9020, abs=0.001)
        assert min(lats) == pytest.approx(40.6637, abs=0.001)
        assert max(lats) == pytest.approx(40.6658, abs=0.001)

    def test_invalid_depth_raises(self):
        _skip_if_absent(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx")
        with pytest.raises(ValueError, match="depth_cm must be one of"):
            parse_tdr_xlsx(str(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=15)


class TestParseDemGeotiff:
    def test_real_data_crs_and_relief(self):
        _skip_if_absent(DATA_DIR / "shalehills_dem_3m_nad83utm18n.tif")
        dem = parse_dem_geotiff(str(DATA_DIR / "shalehills_dem_3m_nad83utm18n.tif"))
        assert dem.cellsize == pytest.approx(3.0)
        relief = np.nanmax(dem.elevation) - np.nanmin(dem.elevation)
        # Paper's own text (Section 4.2.2): "The topography spans about
        # 70m" -- confirmed against the real reprojected DEM.
        assert relief == pytest.approx(68.5, abs=1.0)

    def test_wrong_crs_raises_loudly(self, tmp_path):
        """A DEM in the ORIGINAL (NAD27) CRS must be rejected, not
        silently used -- see module docstring for why a silent NAD27/
        NAD83 mixup would be catastrophic at this site's scale."""
        import rasterio
        from rasterio.transform import from_origin

        bad_path = tmp_path / "wrong_crs.tif"
        transform = from_origin(0, 10, 1, 1)
        with rasterio.open(
            bad_path, "w", driver="GTiff", height=10, width=10, count=1,
            dtype="float32", crs="EPSG:26718", transform=transform,
        ) as dst:
            dst.write(np.zeros((10, 10), dtype="float32"), 1)
        with pytest.raises(ValueError, match="EPSG:26918"):
            parse_dem_geotiff(str(bad_path))


class TestSsurgo:
    def test_real_data_chorizon_sand_silt_clay_sum_to_100(self):
        """Strongest possible confirmation the real MDB-derived column
        schema (Session 10) is correctly aligned: a real horizon's
        sandtotal_r + silttotal_r + claytotal_r must sum to ~100."""
        _skip_if_absent(DATA_DIR / "ssurgo" / "chorizon.txt")
        horizons = parse_ssurgo_chorizon(str(DATA_DIR / "ssurgo" / "chorizon.txt"))
        checked = 0
        for h in horizons:
            if h["sandtotal_r"] and h["silttotal_r"] and h["claytotal_r"]:
                total = float(h["sandtotal_r"]) + float(h["silttotal_r"]) + float(h["claytotal_r"])
                assert total == pytest.approx(100.0, abs=0.5)
                checked += 1
        assert checked > 0

    def test_real_data_mapunit_names_match_known_shale_hills_soil_series(self):
        """Berks and Weikert are the canonical, extensively-studied soil
        series at this specific site in the Critical Zone literature --
        their presence here is itself a strong correctness signal, not
        just a schema check."""
        _skip_if_absent(DATA_DIR / "ssurgo" / "mapunit.txt")
        mapunits = parse_ssurgo_mapunit(str(DATA_DIR / "ssurgo" / "mapunit.txt"))
        names = " ".join(m["muname"] for m in mapunits)
        assert "Berks" in names
        assert "Weikert" in names

    def test_dominant_component_texture_by_mukey_real_data(self):
        _skip_if_absent(
            DATA_DIR / "ssurgo" / "mapunit.txt", DATA_DIR / "ssurgo" / "comp.txt", DATA_DIR / "ssurgo" / "chorizon.txt"
        )
        texture = dominant_component_texture_by_mukey(
            str(DATA_DIR / "ssurgo" / "mapunit.txt"),
            str(DATA_DIR / "ssurgo" / "comp.txt"),
            str(DATA_DIR / "ssurgo" / "chorizon.txt"),
        )
        assert len(texture) > 0
        for sand_pct, clay_pct in texture.values():
            assert 0 <= sand_pct <= 100
            assert 0 <= clay_pct <= 100

    def test_assign_mukey_to_points_real_data(self):
        _skip_if_absent(DATA_DIR / "ssurgo" / "soilmu_subset.shp", DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx")
        readings = parse_tdr_xlsx(str(DATA_DIR / "SSHCZO_2010TDRSoilMoisture.xlsx"), depth_cm=10)
        xy = np.array([(r.x, r.y) for r in readings[:10]])
        polygons = parse_ssurgo_mapunit_polygons(str(DATA_DIR / "ssurgo" / "soilmu_subset.shp"))
        mukeys = assign_mukey_to_points(polygons, xy)
        assert len(mukeys) == 10
        assert all(mk is not None for mk in mukeys)


class TestParseMeteoFile:
    def test_real_data_toa5_format(self):
        path = DATA_DIR / "met" / "2010_CZO_FluxTowerMeteo.dat"
        _skip_if_absent(path)
        records = parse_meteo_file(str(path))
        assert len(records) > 40000  # 10-minute intervals across a full year
        assert records[0].timestamp.year == 2010

    def test_real_data_rejects_implausible_pressure(self):
        """Real sensor dropouts exist in the raw files (e.g. a spurious
        ~11 kPa reading, physically impossible at this site's elevation)
        -- must be rejected, not silently fed into FAO-56."""
        path = DATA_DIR / "met" / "2011_CZO_FluxTowerMeteo.dat"
        _skip_if_absent(path)
        records = parse_meteo_file(str(path))
        for r in records:
            if r.pressure_kpa is not None:
                assert PLAUSIBLE_PRESSURE_KPA[0] <= r.pressure_kpa <= PLAUSIBLE_PRESSURE_KPA[1]

    def test_synthetic_toa5_parsing(self, tmp_path):
        content = (
            "TOA5,CZO_EC1,CR1000,4745,CR1000.Std.16,CPU:CZO_EC_d2.cr1,61052,ten_min,,,\n"
            "TIMESTAMP,RECORD,pressure_irga_mean,h2o_irga_mean,h2o_hmp_mean,LWS_current,"
            "T_hmp_mean,T_hmp_current,RH_hmp_current,net_radiation_mean,PAR_mean\n"
            "TS,RN,kPa,g/(m^3),g/(m^3),mV,C,C,%,W/(m^2),umol/(m^2 s)\n"
            ",,Avg,Avg,Avg,Smp,Avg,Smp,Smp,Avg,Avg\n"
            "06/15/2010 12:00,1,97.5,5.0,4.0,260.0,20.5,20.6,65.0,450.2,800.0\n"
        )
        path = tmp_path / "test.dat"
        path.write_text(content)
        records = parse_meteo_file(str(path))
        assert len(records) == 1
        r = records[0]
        assert r.pressure_kpa == pytest.approx(97.5)
        assert r.temperature_c == pytest.approx(20.5)
        assert r.relative_humidity_pct == pytest.approx(65.0)
        assert r.net_radiation_w_m2 == pytest.approx(450.2)

    def test_synthetic_toa5_handles_nan_fields(self, tmp_path):
        content = (
            "TOA5,X,X,X,X,X,X,X,,,\n"
            "TIMESTAMP,RECORD,pressure_irga_mean,h2o_irga_mean,h2o_hmp_mean,LWS_current,"
            "T_hmp_mean,T_hmp_current,RH_hmp_current,net_radiation_mean,PAR_mean\n"
            "TS,RN,kPa,g/(m^3),g/(m^3),mV,C,C,%,W/(m^2),umol/(m^2 s)\n"
            ",,Avg,Avg,Avg,Smp,Avg,Smp,Smp,Avg,Avg\n"
            "05/20/2013 16:20,1,NAN,NAN,13.09,262.7,25.57,25.44,55.72,112.9,357\n"
        )
        path = tmp_path / "test.dat"
        path.write_text(content)
        records = parse_meteo_file(str(path))
        assert records[0].pressure_kpa is None
