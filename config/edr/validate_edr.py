#!/usr/bin/env python3
"""
EDR Configuration Validator

Validates EDR (Environmental Data Retrieval) YAML configuration files.

File types validated:
- Collection configs (gfs.yaml, hrrr.yaml, etc.): Define EDR collections and parameters
- server.yaml: Global server configuration
- locations.yaml: Named location definitions

Usage:
    ./validate_edr.py                    # Validate all *.yaml files
    ./validate_edr.py gfs.yaml           # Validate specific file
    ./validate_edr.py --help             # Show help

Exit codes:
    0 - All files valid
    1 - Validation errors found
    2 - No YAML files found
"""

import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML is required. Install with: pip install pyyaml")
    sys.exit(1)


# =============================================================================
# Valid values for enumerated fields
# =============================================================================

VALID_RUN_MODES = {"instances", "latest"}

VALID_DATA_TYPES = {
    "forecast",
    "observation",
    "pointobservation",
    "pointforecast",
    "static",
}

VALID_OUTPUT_FORMATS = {
    "application/vnd.cov+json",
    "application/geo+json",
    "image/png",
    "application/json",
}

VALID_CRS = {"CRS:84", "EPSG:4326", "EPSG:3857"}

VALID_LEVEL_TYPES = {
    "surface",
    "height_above_ground",
    "height_above_ground_layer",
    "height_above_msl",
    "isobaric",
    "mean_sea_level",
    "entire_atmosphere",
    "low_cloud_layer",
    "middle_cloud_layer",
    "high_cloud_layer",
    "cloud_layer",
    "cloud_base",
    "cloud_top",
    "top_of_atmosphere",
    "depth_below_surface",
    "boundary_layer",
    "tropopause",
    "mixed",  # For collections with mixed level types
}


class ValidationError:
    """Represents a single validation error."""

    def __init__(self, path: str, message: str, severity: str = "error"):
        self.path = path
        self.message = message
        self.severity = severity

    def __str__(self):
        icon = "ERROR" if self.severity == "error" else "WARNING"
        return f"  [{icon}] {self.path}: {self.message}"


class EDRValidator:
    """Validates EDR configuration YAML files."""

    def __init__(self, filename: str):
        self.filename = filename
        self.errors: list[ValidationError] = []
        self.warnings: list[ValidationError] = []
        self.data: dict = {}

    def add_error(self, path: str, message: str):
        self.errors.append(ValidationError(path, message, "error"))

    def add_warning(self, path: str, message: str):
        self.warnings.append(ValidationError(path, message, "warning"))

    def validate(self) -> bool:
        """Validate the YAML file. Returns True if valid (no errors)."""
        # Load YAML
        try:
            with open(self.filename, "r") as f:
                self.data = yaml.safe_load(f)
        except yaml.YAMLError as e:
            self.add_error("(file)", f"Invalid YAML syntax: {e}")
            return False
        except FileNotFoundError:
            self.add_error("(file)", f"File not found: {self.filename}")
            return False

        if not isinstance(self.data, dict):
            self.add_error("(root)", "Root must be a YAML mapping/dictionary")
            return False

        # Determine file type and validate accordingly
        filepath = Path(self.filename)

        if filepath.name == "server.yaml":
            self._validate_server_config()
        elif filepath.name == "locations.yaml":
            self._validate_locations_config()
        else:
            self._validate_collection_config()

        return len(self.errors) == 0

    def _validate_server_config(self):
        """Validate server.yaml configuration."""
        if "global_limits" not in self.data:
            self.add_warning("global_limits", "Missing 'global_limits' section")
            return

        limits = self.data["global_limits"]
        if not isinstance(limits, dict):
            self.add_error("global_limits", "Must be a mapping")
            return

        # Validate numeric limit fields
        numeric_fields = [
            "max_collections_per_location_request",
            "max_location_response_size_mb",
        ]
        for field in numeric_fields:
            if field in limits:
                if not isinstance(limits[field], (int, float)) or limits[field] <= 0:
                    self.add_error(
                        f"global_limits.{field}", "Must be a positive number"
                    )

    def _validate_locations_config(self):
        """Validate locations.yaml configuration."""
        if "locations" not in self.data:
            self.add_error("locations", "Missing required field 'locations'")
            return

        locations = self.data["locations"]
        if not isinstance(locations, list):
            self.add_error("locations", "Must be a list")
            return

        seen_ids = set()
        for i, loc in enumerate(locations):
            path = f"locations[{i}]"
            if not isinstance(loc, dict):
                self.add_error(path, "Each location must be a mapping")
                continue

            # Required fields
            if "id" not in loc:
                self.add_error(f"{path}.id", "Missing required field 'id'")
            else:
                loc_id = loc["id"]
                if loc_id in seen_ids:
                    self.add_error(f"{path}.id", f"Duplicate location ID '{loc_id}'")
                seen_ids.add(loc_id)

            if "name" not in loc:
                self.add_error(f"{path}.name", "Missing required field 'name'")

            if "coords" not in loc:
                self.add_error(f"{path}.coords", "Missing required field 'coords'")
            else:
                coords = loc["coords"]
                if not isinstance(coords, list) or len(coords) != 2:
                    self.add_error(
                        f"{path}.coords", "Must be [longitude, latitude] array"
                    )
                else:
                    lon, lat = coords
                    if not isinstance(lon, (int, float)):
                        self.add_error(
                            f"{path}.coords[0]", "Longitude must be a number"
                        )
                    elif lon < -180 or lon > 180:
                        self.add_error(
                            f"{path}.coords[0]",
                            f"Longitude {lon} out of range [-180, 180]",
                        )
                    if not isinstance(lat, (int, float)):
                        self.add_error(f"{path}.coords[1]", "Latitude must be a number")
                    elif lat < -90 or lat > 90:
                        self.add_error(
                            f"{path}.coords[1]",
                            f"Latitude {lat} out of range [-90, 90]",
                        )

    def _validate_collection_config(self):
        """Validate a model collection config (gfs.yaml, hrrr.yaml, etc.)."""
        # Required: model
        if "model" not in self.data:
            self.add_error("model", "Missing required field 'model'")
        elif not isinstance(self.data["model"], str):
            self.add_error("model", "Must be a string")

        # Optional: data_type
        if "data_type" in self.data:
            if self.data["data_type"] not in VALID_DATA_TYPES:
                self.add_warning(
                    "data_type",
                    f"Unknown data_type '{self.data['data_type']}'. "
                    f"Valid types: {', '.join(sorted(VALID_DATA_TYPES))}",
                )

        # Required: collections
        if "collections" not in self.data:
            self.add_error("collections", "Missing required field 'collections'")
            return

        collections = self.data["collections"]
        if not isinstance(collections, list):
            self.add_error("collections", "Must be a list")
            return

        if len(collections) == 0:
            self.add_error("collections", "Must have at least one collection")
            return

        seen_ids = set()
        for i, coll in enumerate(collections):
            self._validate_collection(coll, i, seen_ids)

        # Optional: settings
        if "settings" in self.data:
            self._validate_settings(self.data["settings"])

        # Optional: limits
        if "limits" in self.data:
            self._validate_limits(self.data["limits"])

    def _validate_collection(self, coll: Any, index: int, seen_ids: set):
        """Validate a single collection definition."""
        path = f"collections[{index}]"

        if not isinstance(coll, dict):
            self.add_error(path, "Each collection must be a mapping")
            return

        # Required: id
        if "id" not in coll:
            self.add_error(f"{path}.id", "Missing required field 'id'")
        else:
            coll_id = coll["id"]
            if not isinstance(coll_id, str):
                self.add_error(f"{path}.id", "Must be a string")
            elif coll_id in seen_ids:
                self.add_error(f"{path}.id", f"Duplicate collection ID '{coll_id}'")
            else:
                seen_ids.add(coll_id)

        # Required: title
        if "title" not in coll:
            self.add_error(f"{path}.title", "Missing required field 'title'")
        elif not isinstance(coll["title"], str):
            self.add_error(f"{path}.title", "Must be a string")

        # Optional: description
        if "description" in coll and not isinstance(coll["description"], str):
            self.add_error(f"{path}.description", "Must be a string")

        # Required: parameters
        if "parameters" not in coll:
            self.add_error(f"{path}.parameters", "Missing required field 'parameters'")
        else:
            self._validate_parameters(coll["parameters"], f"{path}.parameters")

        # Required: run_mode
        if "run_mode" not in coll:
            self.add_error(f"{path}.run_mode", "Missing required field 'run_mode'")
        elif coll["run_mode"] not in VALID_RUN_MODES:
            self.add_error(
                f"{path}.run_mode",
                f"Invalid run_mode '{coll['run_mode']}'. "
                f"Valid modes: {', '.join(sorted(VALID_RUN_MODES))}",
            )

        # Optional: level_filter
        if "level_filter" in coll:
            self._validate_level_filter(coll["level_filter"], f"{path}.level_filter")

    def _validate_parameters(self, params: Any, path: str):
        """Validate the parameters array."""
        if not isinstance(params, list):
            self.add_error(path, "Must be a list")
            return

        if len(params) == 0:
            self.add_error(path, "Must have at least one parameter")
            return

        for i, param in enumerate(params):
            param_path = f"{path}[{i}]"
            if not isinstance(param, dict):
                self.add_error(param_path, "Each parameter must be a mapping")
                continue

            # Required: name
            if "name" not in param:
                self.add_error(f"{param_path}.name", "Missing required field 'name'")
            elif not isinstance(param["name"], str):
                self.add_error(f"{param_path}.name", "Must be a string")

            # Required: levels
            if "levels" not in param:
                self.add_error(
                    f"{param_path}.levels", "Missing required field 'levels'"
                )
            elif not isinstance(param["levels"], list):
                self.add_error(f"{param_path}.levels", "Must be a list")
            elif len(param["levels"]) == 0:
                self.add_error(f"{param_path}.levels", "Must have at least one level")

            # Optional: valid_range
            if "valid_range" in param:
                self._validate_range(param["valid_range"], f"{param_path}.valid_range")

    def _validate_range(self, range_obj: Any, path: str):
        """Validate a valid_range object."""
        if not isinstance(range_obj, dict):
            self.add_error(path, "Must be a mapping with 'min' and 'max'")
            return

        if "min" not in range_obj:
            self.add_error(f"{path}.min", "Missing required field 'min'")
        elif not isinstance(range_obj["min"], (int, float)):
            self.add_error(f"{path}.min", "Must be a number")

        if "max" not in range_obj:
            self.add_error(f"{path}.max", "Missing required field 'max'")
        elif not isinstance(range_obj["max"], (int, float)):
            self.add_error(f"{path}.max", "Must be a number")

        # Check min < max
        if (
            "min" in range_obj
            and "max" in range_obj
            and isinstance(range_obj["min"], (int, float))
            and isinstance(range_obj["max"], (int, float))
        ):
            if range_obj["min"] >= range_obj["max"]:
                self.add_error(
                    path,
                    f"min ({range_obj['min']}) must be less than max ({range_obj['max']})",
                )

    def _validate_level_filter(self, lf: Any, path: str):
        """Validate a level_filter object."""
        if not isinstance(lf, dict):
            self.add_error(path, "Must be a mapping")
            return

        if "level_type" in lf:
            if lf["level_type"] not in VALID_LEVEL_TYPES:
                self.add_warning(
                    f"{path}.level_type",
                    f"Unknown level_type '{lf['level_type']}'. "
                    f"Known types: {', '.join(sorted(VALID_LEVEL_TYPES))}",
                )

        # level_code or level_codes should be integers
        if "level_code" in lf and not isinstance(lf["level_code"], int):
            self.add_error(f"{path}.level_code", "Must be an integer")

        if "level_codes" in lf:
            if not isinstance(lf["level_codes"], list):
                self.add_error(f"{path}.level_codes", "Must be a list of integers")
            else:
                for i, code in enumerate(lf["level_codes"]):
                    if not isinstance(code, int):
                        self.add_error(f"{path}.level_codes[{i}]", "Must be an integer")

    def _validate_settings(self, settings: Any):
        """Validate the settings section."""
        path = "settings"
        if not isinstance(settings, dict):
            self.add_error(path, "Must be a mapping")
            return

        # output_formats
        if "output_formats" in settings:
            formats = settings["output_formats"]
            if not isinstance(formats, list):
                self.add_error(f"{path}.output_formats", "Must be a list")
            else:
                for i, fmt in enumerate(formats):
                    if fmt not in VALID_OUTPUT_FORMATS:
                        self.add_warning(
                            f"{path}.output_formats[{i}]",
                            f"Unknown format '{fmt}'. "
                            f"Known formats: {', '.join(sorted(VALID_OUTPUT_FORMATS))}",
                        )

        # default_crs
        if "default_crs" in settings:
            if settings["default_crs"] not in VALID_CRS:
                self.add_warning(
                    f"{path}.default_crs",
                    f"Unknown CRS '{settings['default_crs']}'. "
                    f"Known CRS: {', '.join(sorted(VALID_CRS))}",
                )

        # supported_crs
        if "supported_crs" in settings:
            crs_list = settings["supported_crs"]
            if not isinstance(crs_list, list):
                self.add_error(f"{path}.supported_crs", "Must be a list")
            else:
                for i, crs in enumerate(crs_list):
                    if crs not in VALID_CRS:
                        self.add_warning(
                            f"{path}.supported_crs[{i}]",
                            f"Unknown CRS '{crs}'",
                        )

    def _validate_limits(self, limits: Any):
        """Validate the limits section."""
        path = "limits"
        if not isinstance(limits, dict):
            self.add_error(path, "Must be a mapping")
            return

        # All limit fields should be positive numbers
        numeric_fields = [
            "max_parameters_per_request",
            "max_time_steps",
            "max_vertical_levels",
            "max_response_size_mb",
            "max_area_sq_degrees",
            "max_area_sq_degrees_png",
            "max_radius_km",
        ]
        for field in numeric_fields:
            if field in limits:
                val = limits[field]
                if not isinstance(val, (int, float)):
                    self.add_error(f"{path}.{field}", "Must be a number")
                elif val <= 0:
                    self.add_error(f"{path}.{field}", "Must be a positive number")


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Validate EDR configuration YAML files",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "files",
        nargs="*",
        help="YAML files to validate (default: all *.yaml in current directory)",
    )
    parser.add_argument(
        "-q", "--quiet", action="store_true", help="Only show errors, not warnings"
    )

    args = parser.parse_args()

    # Determine files to validate
    if args.files:
        files = [Path(f) for f in args.files]
    else:
        script_dir = Path(__file__).parent
        files = sorted(script_dir.glob("*.yaml"))

    if not files:
        print("No YAML files found to validate")
        sys.exit(2)

    # Validate each file
    total_errors = 0
    total_warnings = 0
    valid_count = 0

    for filepath in files:
        validator = EDRValidator(str(filepath))
        is_valid = validator.validate()

        if is_valid:
            valid_count += 1
            print(f"OK {filepath.name}")
            if validator.warnings and not args.quiet:
                for warning in validator.warnings:
                    print(warning)
        else:
            print(f"INVALID {filepath.name}")
            for error in validator.errors:
                print(error)
            total_errors += len(validator.errors)

        if validator.warnings and not args.quiet and not is_valid:
            for warning in validator.warnings:
                print(warning)

        total_warnings += len(validator.warnings)

    # Summary
    print()
    if total_errors == 0:
        print(f"All {len(files)} EDR configuration(s) valid")
        if total_warnings > 0 and not args.quiet:
            print(f"  ({total_warnings} warning(s))")
        sys.exit(0)
    else:
        print(
            f"Validation failed: {total_errors} error(s) in {len(files) - valid_count} file(s)"
        )
        if total_warnings > 0 and not args.quiet:
            print(f"  ({total_warnings} warning(s))")
        sys.exit(1)


if __name__ == "__main__":
    main()
