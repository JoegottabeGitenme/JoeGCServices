#!/usr/bin/env python3
"""
Model Configuration Validator

Validates model YAML configuration files against the expected schema.
Run from the config/models directory or pass paths to YAML files.

Usage:
    ./validate.py                    # Validate all *.yaml files in current directory
    ./validate.py gfs.yaml           # Validate specific file
    ./validate.py *.yaml             # Validate multiple files
    ./validate.py --help             # Show help

Exit codes:
    0 - All files valid
    1 - Validation errors found
    2 - No YAML files found
"""

import sys
import os
import re
from pathlib import Path
from typing import Any

# Try to import yaml, with helpful error if not installed
try:
    import yaml
except ImportError:
    print("ERROR: PyYAML is required. Install with: pip install pyyaml")
    sys.exit(1)


# =============================================================================
# Schema Definition
# =============================================================================

# Valid values for enumerated fields
VALID_DIMENSION_TYPES = {"forecast", "observation"}
VALID_SOURCE_TYPES = {"aws_s3", "aws_s3_goes", "aws_s3_grib2", "local", "http"}
VALID_PROJECTION_TYPES = {
    "geographic",
    "latlon",
    "geostationary",
    "lambert_conformal",
    "mercator",
}
VALID_SCHEDULE_TYPES = {"forecast", "observation"}
VALID_LEVEL_TYPES = {
    "surface",
    "height_above_ground",
    "height_above_ground_layer",  # For layer-averaged data (e.g., 0-6km)
    "isobaric",
    "mean_sea_level",
    "entire_atmosphere",
    "low_cloud_layer",
    "middle_cloud_layer",
    "high_cloud_layer",
    "cloud_top",
    "top_of_atmosphere",
    "depth_below_surface",
    "boundary_layer",
    "tropopause",
}
VALID_STYLES = {
    "default",
    "temperature",
    "wind",
    "precipitation",
    "humidity",
    "atmospheric",
    "cape",
    "cloud",
    "visibility",
    "reflectivity",
    "precip_rate",
    "goes_visible",
    "goes_ir",
    "wind_barbs",
    "helicity",
    "lightning",
    "smoke",
    "radar",
}
VALID_CONVERSIONS = {
    "K_to_C",
    "K_to_F",
    "Pa_to_hPa",
    "Pa_to_mb",
    "m_to_km",
    "m_to_ft",
    "m_to_kft",  # meters to kilofeet (for cloud tops)
    "ms_to_kt",
    "ms_to_mph",
}


class ValidationError:
    """Represents a single validation error."""

    def __init__(self, path: str, message: str, severity: str = "error"):
        self.path = path
        self.message = message
        self.severity = severity  # "error" or "warning"

    def __str__(self):
        icon = "ERROR" if self.severity == "error" else "WARNING"
        return f"  [{icon}] {self.path}: {self.message}"


class ModelValidator:
    """Validates model configuration YAML files."""

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

        # Validate each section
        self._validate_model_section()
        self._validate_dimensions_section()
        self._validate_source_section()
        self._validate_grid_section()
        self._validate_schedule_section()
        self._validate_retention_section()
        self._validate_precaching_section()
        self._validate_parameters_section()
        self._validate_composites_section()

        return len(self.errors) == 0

    def _validate_model_section(self):
        """Validate the 'model' section (required)."""
        if "model" not in self.data:
            self.add_error("model", "Missing required section 'model'")
            return

        model = self.data["model"]
        if not isinstance(model, dict):
            self.add_error("model", "Section must be a mapping")
            return

        # Required fields
        self._require_string(
            model,
            "model.id",
            r"^[a-z][a-z0-9_]*$",
            "Must be lowercase alphanumeric with underscores, starting with letter",
        )
        self._require_string(model, "model.name")

        # Optional fields
        self._optional_string(model, "model.description")
        self._optional_bool(model, "model.enabled")

    def _validate_dimensions_section(self):
        """Validate the 'dimensions' section (recommended)."""
        if "dimensions" not in self.data:
            self.add_warning(
                "dimensions",
                "Missing 'dimensions' section - will infer from schedule.type",
            )
            return

        dims = self.data["dimensions"]
        if not isinstance(dims, dict):
            self.add_error("dimensions", "Section must be a mapping")
            return

        # Validate type
        dim_type = dims.get("type")
        if dim_type is None:
            self.add_warning(
                "dimensions.type", "Missing 'type' - will default to 'forecast'"
            )
        elif dim_type not in VALID_DIMENSION_TYPES:
            self.add_error(
                "dimensions.type",
                f"Invalid type '{dim_type}'. Must be one of: {', '.join(sorted(VALID_DIMENSION_TYPES))}",
            )

        # Type-specific validation
        if dim_type == "forecast":
            # Forecast models should have run and forecast dimensions
            if dims.get("time"):
                self.add_warning(
                    "dimensions.time",
                    "Forecast models typically don't use TIME dimension (use RUN + FORECAST)",
                )
        elif dim_type == "observation":
            # Observation models should have time dimension
            if dims.get("run") or dims.get("forecast"):
                self.add_warning(
                    "dimensions.run/forecast",
                    "Observation models typically don't use RUN/FORECAST dimensions (use TIME)",
                )

        # All dimension flags should be boolean
        for field in ["run", "forecast", "time", "elevation"]:
            if field in dims and not isinstance(dims[field], bool):
                self.add_error(f"dimensions.{field}", f"Must be a boolean (true/false)")

    def _validate_source_section(self):
        """Validate the 'source' section (required)."""
        if "source" not in self.data:
            self.add_error("source", "Missing required section 'source'")
            return

        source = self.data["source"]
        if not isinstance(source, dict):
            self.add_error("source", "Section must be a mapping")
            return

        # Required: type
        source_type = source.get("type")
        if source_type is None:
            self.add_error("source.type", "Missing required field 'type'")
        elif source_type not in VALID_SOURCE_TYPES:
            self.add_error(
                "source.type",
                f"Invalid type '{source_type}'. Must be one of: {', '.join(sorted(VALID_SOURCE_TYPES))}",
            )

        # AWS S3 sources need bucket
        if source_type and source_type.startswith("aws_s3"):
            if "bucket" not in source:
                self.add_error(
                    "source.bucket", "Missing required field 'bucket' for AWS S3 source"
                )
            if "region" not in source:
                self.add_warning(
                    "source.region", "Missing 'region' - will default to us-east-1"
                )

    def _validate_grid_section(self):
        """Validate the 'grid' section (required)."""
        if "grid" not in self.data:
            self.add_error("grid", "Missing required section 'grid'")
            return

        grid = self.data["grid"]
        if not isinstance(grid, dict):
            self.add_error("grid", "Section must be a mapping")
            return

        # Required: projection
        projection = grid.get("projection")
        if projection is None:
            self.add_error("grid.projection", "Missing required field 'projection'")
        elif projection not in VALID_PROJECTION_TYPES:
            self.add_error(
                "grid.projection",
                f"Invalid projection '{projection}'. Must be one of: {', '.join(sorted(VALID_PROJECTION_TYPES))}",
            )

        # Validate bbox if present
        if "bbox" in grid:
            bbox = grid["bbox"]
            if not isinstance(bbox, dict):
                self.add_error(
                    "grid.bbox",
                    "Must be a mapping with min_lon, min_lat, max_lon, max_lat",
                )
            else:
                for field in ["min_lon", "min_lat", "max_lon", "max_lat"]:
                    if field not in bbox:
                        self.add_error(
                            f"grid.bbox.{field}", f"Missing required field '{field}'"
                        )
                    elif not isinstance(bbox[field], (int, float)):
                        self.add_error(f"grid.bbox.{field}", "Must be a number")

                # Validate ranges
                if all(f in bbox for f in ["min_lon", "max_lon"]):
                    if bbox["min_lon"] >= bbox["max_lon"]:
                        self.add_error("grid.bbox", "min_lon must be less than max_lon")
                if all(f in bbox for f in ["min_lat", "max_lat"]):
                    if bbox["min_lat"] >= bbox["max_lat"]:
                        self.add_error("grid.bbox", "min_lat must be less than max_lat")

        # Geostationary projection needs projection_params
        if projection == "geostationary" and "projection_params" not in grid:
            self.add_error(
                "grid.projection_params",
                "Missing required 'projection_params' for geostationary projection",
            )

    def _validate_schedule_section(self):
        """Validate the 'schedule' section (required)."""
        if "schedule" not in self.data:
            self.add_error("schedule", "Missing required section 'schedule'")
            return

        schedule = self.data["schedule"]
        if not isinstance(schedule, dict):
            self.add_error("schedule", "Section must be a mapping")
            return

        schedule_type = schedule.get("type")

        # Forecast schedules need cycles and forecast_hours
        if schedule_type != "observation":
            if "cycles" in schedule:
                cycles = schedule["cycles"]
                if not isinstance(cycles, list):
                    self.add_error("schedule.cycles", "Must be a list of hours (0-23)")
                else:
                    for i, cycle in enumerate(cycles):
                        if not isinstance(cycle, int) or cycle < 0 or cycle > 23:
                            self.add_error(
                                f"schedule.cycles[{i}]",
                                f"Invalid cycle hour: {cycle}. Must be 0-23",
                            )

            if "forecast_hours" in schedule:
                fh = schedule["forecast_hours"]
                if isinstance(fh, dict):
                    for field in ["start", "end"]:
                        if field not in fh:
                            self.add_error(
                                f"schedule.forecast_hours.{field}",
                                f"Missing required field '{field}'",
                            )
                        elif not isinstance(fh[field], int):
                            self.add_error(
                                f"schedule.forecast_hours.{field}", "Must be an integer"
                            )
                elif not isinstance(fh, list):
                    self.add_error(
                        "schedule.forecast_hours",
                        "Must be a list or mapping with start/end/step",
                    )

        # poll_interval_secs should be positive
        if "poll_interval_secs" in schedule:
            poll = schedule["poll_interval_secs"]
            if not isinstance(poll, int) or poll <= 0:
                self.add_error(
                    "schedule.poll_interval_secs", "Must be a positive integer"
                )

    def _validate_retention_section(self):
        """Validate the 'retention' section (optional but recommended)."""
        if "retention" not in self.data:
            self.add_warning(
                "retention",
                "Missing 'retention' section - data will be kept indefinitely",
            )
            return

        retention = self.data["retention"]
        if not isinstance(retention, dict):
            self.add_error("retention", "Section must be a mapping")
            return

        if "hours" in retention:
            hours = retention["hours"]
            if not isinstance(hours, int) or hours <= 0:
                self.add_error("retention.hours", "Must be a positive integer")

    def _validate_precaching_section(self):
        """Validate the 'precaching' section (optional)."""
        if "precaching" not in self.data:
            return  # Optional section

        precaching = self.data["precaching"]
        if not isinstance(precaching, dict):
            self.add_error("precaching", "Section must be a mapping")
            return

        self._optional_bool(precaching, "precaching.enabled")

        if "parameters" in precaching:
            params = precaching["parameters"]
            if not isinstance(params, list):
                self.add_error(
                    "precaching.parameters", "Must be a list of parameter names"
                )

    def _validate_parameters_section(self):
        """Validate the 'parameters' section (required)."""
        if "parameters" not in self.data:
            self.add_error("parameters", "Missing required section 'parameters'")
            return

        params = self.data["parameters"]
        if not isinstance(params, list):
            self.add_error("parameters", "Section must be a list")
            return

        if len(params) == 0:
            self.add_error("parameters", "Must have at least one parameter defined")
            return

        seen_params = set()
        for i, param in enumerate(params):
            path = f"parameters[{i}]"
            if not isinstance(param, dict):
                self.add_error(path, "Each parameter must be a mapping")
                continue

            # Required: name
            name = param.get("name")
            if name is None:
                self.add_error(f"{path}.name", "Missing required field 'name'")
            elif not isinstance(name, str):
                self.add_error(f"{path}.name", "Must be a string")

            # Warn on duplicates (but they're allowed with different levels)
            if name in seen_params:
                # This is actually OK - same param can have different level sets
                pass
            seen_params.add(name)

            # Optional: description
            self._optional_string(param, f"{path}.description")

            # Required: levels
            if "levels" not in param:
                self.add_error(f"{path}.levels", "Missing required field 'levels'")
            else:
                self._validate_levels(param["levels"], f"{path}.levels")

            # Optional: style (but recommended)
            if "style" in param:
                style = param["style"]
                if style not in VALID_STYLES:
                    self.add_warning(
                        f"{path}.style",
                        f"Unknown style '{style}'. Known styles: {', '.join(sorted(VALID_STYLES))}",
                    )

            # Optional: units
            self._optional_string(param, f"{path}.units")
            self._optional_string(param, f"{path}.display_units")

            # Optional: conversion
            if "conversion" in param:
                conv = param["conversion"]
                if conv not in VALID_CONVERSIONS:
                    self.add_warning(
                        f"{path}.conversion",
                        f"Unknown conversion '{conv}'. Known: {', '.join(sorted(VALID_CONVERSIONS))}",
                    )

    def _validate_levels(self, levels: Any, path: str):
        """Validate a levels array."""
        if not isinstance(levels, list):
            self.add_error(path, "Must be a list")
            return

        if len(levels) == 0:
            self.add_error(path, "Must have at least one level defined")
            return

        for i, level in enumerate(levels):
            level_path = f"{path}[{i}]"
            if not isinstance(level, dict):
                self.add_error(level_path, "Each level must be a mapping")
                continue

            level_type = level.get("type")
            if level_type is None:
                self.add_error(f"{level_path}.type", "Missing required field 'type'")
            elif level_type not in VALID_LEVEL_TYPES:
                self.add_warning(
                    f"{level_path}.type",
                    f"Unknown level type '{level_type}'. Known types: {', '.join(sorted(VALID_LEVEL_TYPES))}",
                )

            # Check for value or values
            has_value = "value" in level
            has_values = "values" in level

            if has_values:
                values = level["values"]
                if not isinstance(values, list):
                    self.add_error(f"{level_path}.values", "Must be a list")
                elif len(values) == 0:
                    self.add_error(
                        f"{level_path}.values", "Must have at least one value"
                    )

    def _validate_composites_section(self):
        """Validate the 'composites' section (optional)."""
        if "composites" not in self.data:
            return  # Optional section

        composites = self.data["composites"]
        if not isinstance(composites, list):
            self.add_error("composites", "Section must be a list")
            return

        # Get defined parameter names for cross-reference
        defined_params = set()
        for param in self.data.get("parameters", []):
            if isinstance(param, dict) and "name" in param:
                defined_params.add(param["name"])

        for i, comp in enumerate(composites):
            path = f"composites[{i}]"
            if not isinstance(comp, dict):
                self.add_error(path, "Each composite must be a mapping")
                continue

            # Required: name
            if "name" not in comp:
                self.add_error(f"{path}.name", "Missing required field 'name'")

            # Required: requires
            if "requires" not in comp:
                self.add_error(f"{path}.requires", "Missing required field 'requires'")
            else:
                requires = comp["requires"]
                if not isinstance(requires, list):
                    self.add_error(
                        f"{path}.requires", "Must be a list of parameter names"
                    )
                else:
                    for req in requires:
                        if req not in defined_params:
                            self.add_warning(
                                f"{path}.requires",
                                f"Required parameter '{req}' not defined in parameters section",
                            )

    # =========================================================================
    # Helper methods
    # =========================================================================

    def _require_string(
        self,
        obj: dict,
        path: str,
        pattern: str | None = None,
        pattern_desc: str | None = None,
    ):
        """Validate a required string field."""
        field = path.split(".")[-1]
        if field not in obj:
            self.add_error(path, f"Missing required field '{field}'")
            return

        value = obj[field]
        if not isinstance(value, str):
            self.add_error(path, "Must be a string")
            return

        if pattern and not re.match(pattern, value):
            desc = pattern_desc or f"Must match pattern: {pattern}"
            self.add_error(path, desc)

    def _optional_string(self, obj: dict, path: str):
        """Validate an optional string field."""
        field = path.split(".")[-1]
        if field in obj and not isinstance(obj[field], str):
            self.add_error(path, "Must be a string")

    def _optional_bool(self, obj: dict, path: str):
        """Validate an optional boolean field."""
        field = path.split(".")[-1]
        if field in obj and not isinstance(obj[field], bool):
            self.add_error(path, "Must be a boolean (true/false)")


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Validate model configuration YAML files",
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
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Show detailed output for valid files",
    )

    args = parser.parse_args()

    # Determine files to validate
    if args.files:
        files = [Path(f) for f in args.files]
    else:
        # Find all YAML files in current directory
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
        # Skip this script if it somehow has .yaml extension
        if filepath.name == "validate.py":
            continue

        validator = ModelValidator(str(filepath))
        is_valid = validator.validate()

        if is_valid:
            valid_count += 1
            if args.verbose:
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
    file_count = len([f for f in files if f.name != "validate.py"])
    if total_errors == 0:
        print(f"All {file_count} model configuration(s) valid")
        if total_warnings > 0 and not args.quiet:
            print(f"  ({total_warnings} warning(s))")
        sys.exit(0)
    else:
        print(
            f"Validation failed: {total_errors} error(s) in {file_count - valid_count} file(s)"
        )
        if total_warnings > 0 and not args.quiet:
            print(f"  ({total_warnings} warning(s))")
        sys.exit(1)


if __name__ == "__main__":
    main()
