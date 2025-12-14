#!/usr/bin/env python3
"""
Style JSON Validation Script

Validates all style JSON files in config/styles/ against the schema defined
in schema.example.json.

Usage:
    python validate_styles.py [--verbose]

Exit codes:
    0 - All files valid
    1 - Validation errors found
"""

import json
import os
import re
import sys
from pathlib import Path
from typing import Any

# Valid style types
VALID_STYLE_TYPES = {
    "gradient",
    "contour",
    "filled_contour",
    "wind_barbs",
    "wind_arrows",
    "numbers",
}

# Valid transform types
VALID_TRANSFORM_TYPES = {
    "none",
    "linear",
    "pa_to_hpa",
    "mps_to_knots",
    "k_to_c",
    "m_to_km",
}

# Valid interpolation types
VALID_INTERPOLATION_TYPES = {"linear", "step", "nearest"}

# Valid out_of_range types
VALID_OUT_OF_RANGE_TYPES = {"clamp", "extend", "transparent"}

# Hex color pattern
HEX_COLOR_PATTERN = re.compile(r"^#[0-9A-Fa-f]{6}([0-9A-Fa-f]{2})?$")


class ValidationError:
    def __init__(self, file: str, path: str, message: str):
        self.file = file
        self.path = path
        self.message = message

    def __str__(self):
        return f"{self.file}: {self.path}: {self.message}"


def validate_color(color: Any, path: str, errors: list, file: str):
    """Validate a color value."""
    if color == "transparent":
        return
    if not isinstance(color, str):
        errors.append(
            ValidationError(
                file, path, f"Color must be string, got {type(color).__name__}"
            )
        )
        return
    if not HEX_COLOR_PATTERN.match(color):
        errors.append(
            ValidationError(
                file,
                path,
                f"Invalid color format '{color}'. Expected #RRGGBB or #RRGGBBAA",
            )
        )


def validate_stop(stop: Any, index: int, path: str, errors: list, file: str):
    """Validate a color stop."""
    stop_path = f"{path}[{index}]"

    if not isinstance(stop, dict):
        errors.append(
            ValidationError(
                file, stop_path, f"Stop must be object, got {type(stop).__name__}"
            )
        )
        return

    # Required: value
    if "value" not in stop:
        errors.append(
            ValidationError(file, stop_path, "Missing required field 'value'")
        )
    elif not isinstance(stop["value"], (int, float)):
        errors.append(
            ValidationError(
                file,
                f"{stop_path}.value",
                f"Value must be number, got {type(stop['value']).__name__}",
            )
        )

    # Required: color
    if "color" not in stop:
        errors.append(
            ValidationError(file, stop_path, "Missing required field 'color'")
        )
    else:
        validate_color(stop["color"], f"{stop_path}.color", errors, file)

    # Optional: label (string)
    if "label" in stop and not isinstance(stop["label"], str):
        errors.append(
            ValidationError(
                file,
                f"{stop_path}.label",
                f"Label must be string, got {type(stop['label']).__name__}",
            )
        )


def validate_transform(transform: Any, path: str, errors: list, file: str):
    """Validate a transform object."""
    if not isinstance(transform, dict):
        errors.append(
            ValidationError(
                file, path, f"Transform must be object, got {type(transform).__name__}"
            )
        )
        return

    if "type" not in transform:
        errors.append(ValidationError(file, path, "Missing required field 'type'"))
    elif transform["type"] not in VALID_TRANSFORM_TYPES:
        errors.append(
            ValidationError(
                file,
                f"{path}.type",
                f"Invalid transform type '{transform['type']}'. Valid types: {VALID_TRANSFORM_TYPES}",
            )
        )

    # For 'linear' transform, scale/offset are optional numbers
    if transform.get("type") == "linear":
        if "scale" in transform and not isinstance(transform["scale"], (int, float)):
            errors.append(
                ValidationError(file, f"{path}.scale", "Scale must be a number")
            )
        if "offset" in transform and not isinstance(transform["offset"], (int, float)):
            errors.append(
                ValidationError(file, f"{path}.offset", "Offset must be a number")
            )


def validate_range(range_obj: Any, path: str, errors: list, file: str):
    """Validate a range object."""
    if not isinstance(range_obj, dict):
        errors.append(
            ValidationError(
                file, path, f"Range must be object, got {type(range_obj).__name__}"
            )
        )
        return

    if "min" in range_obj and not isinstance(range_obj["min"], (int, float)):
        errors.append(ValidationError(file, f"{path}.min", "Min must be a number"))
    if "max" in range_obj and not isinstance(range_obj["max"], (int, float)):
        errors.append(ValidationError(file, f"{path}.max", "Max must be a number"))

    if "min" in range_obj and "max" in range_obj:
        if isinstance(range_obj["min"], (int, float)) and isinstance(
            range_obj["max"], (int, float)
        ):
            if range_obj["min"] >= range_obj["max"]:
                errors.append(
                    ValidationError(
                        file,
                        path,
                        f"Min ({range_obj['min']}) must be less than max ({range_obj['max']})",
                    )
                )


def validate_legend(legend: Any, path: str, errors: list, file: str):
    """Validate a legend object."""
    if not isinstance(legend, dict):
        errors.append(
            ValidationError(
                file, path, f"Legend must be object, got {type(legend).__name__}"
            )
        )
        return

    if "title" in legend and not isinstance(legend["title"], str):
        errors.append(ValidationError(file, f"{path}.title", "Title must be string"))

    if "labels" in legend:
        if not isinstance(legend["labels"], list):
            errors.append(
                ValidationError(file, f"{path}.labels", "Labels must be array")
            )
        else:
            for i, label in enumerate(legend["labels"]):
                if not isinstance(label, str):
                    errors.append(
                        ValidationError(
                            file,
                            f"{path}.labels[{i}]",
                            f"Label must be string, got {type(label).__name__}",
                        )
                    )


def validate_contour(contour: Any, path: str, errors: list, file: str):
    """Validate contour-specific options."""
    if not isinstance(contour, dict):
        errors.append(
            ValidationError(
                file, path, f"Contour must be object, got {type(contour).__name__}"
            )
        )
        return

    number_fields = [
        "interval",
        "base",
        "min_value",
        "max_value",
        "line_width",
        "major_interval",
        "major_line_width",
        "label_font_size",
        "smoothing_passes",
    ]
    for field in number_fields:
        if field in contour and not isinstance(contour[field], (int, float)):
            errors.append(
                ValidationError(file, f"{path}.{field}", f"Field must be number")
            )

    if "line_color" in contour:
        validate_color(contour["line_color"], f"{path}.line_color", errors, file)

    if "labels" in contour and not isinstance(contour["labels"], bool):
        errors.append(ValidationError(file, f"{path}.labels", "Labels must be boolean"))


def validate_wind(wind: Any, path: str, errors: list, file: str):
    """Validate wind-specific options."""
    if not isinstance(wind, dict):
        errors.append(
            ValidationError(
                file, path, f"Wind must be object, got {type(wind).__name__}"
            )
        )
        return

    number_fields = [
        "spacing",
        "size",
        "line_width",
        "calm_threshold",
        "min_length",
        "max_length",
    ]
    for field in number_fields:
        if field in wind and not isinstance(wind[field], (int, float)):
            errors.append(
                ValidationError(file, f"{path}.{field}", f"Field must be number")
            )

    if "color" in wind:
        validate_color(wind["color"], f"{path}.color", errors, file)

    if "direction_from" in wind and not isinstance(wind["direction_from"], bool):
        errors.append(
            ValidationError(
                file, f"{path}.direction_from", "direction_from must be boolean"
            )
        )


def validate_color_by_speed(cbs: Any, path: str, errors: list, file: str):
    """Validate color_by_speed options."""
    if not isinstance(cbs, dict):
        errors.append(ValidationError(file, path, f"color_by_speed must be object"))
        return

    if "enabled" in cbs and not isinstance(cbs["enabled"], bool):
        errors.append(
            ValidationError(file, f"{path}.enabled", "enabled must be boolean")
        )

    if "stops" in cbs:
        if not isinstance(cbs["stops"], list):
            errors.append(ValidationError(file, f"{path}.stops", "stops must be array"))
        else:
            for i, stop in enumerate(cbs["stops"]):
                validate_stop(stop, i, f"{path}.stops", errors, file)

    if "interpolation" in cbs and cbs["interpolation"] not in VALID_INTERPOLATION_TYPES:
        errors.append(
            ValidationError(
                file,
                f"{path}.interpolation",
                f"Invalid interpolation '{cbs['interpolation']}'. Valid: {VALID_INTERPOLATION_TYPES}",
            )
        )


def validate_numbers(numbers: Any, path: str, errors: list, file: str):
    """Validate numbers-specific options."""
    if not isinstance(numbers, dict):
        errors.append(
            ValidationError(
                file, path, f"Numbers must be object, got {type(numbers).__name__}"
            )
        )
        return

    number_fields = [
        "spacing",
        "font_size",
        "decimal_places",
    ]
    for field in number_fields:
        if field in numbers and not isinstance(numbers[field], (int, float)):
            errors.append(
                ValidationError(file, f"{path}.{field}", f"Field must be number")
            )

    color_fields = ["font_color", "background_color"]
    for field in color_fields:
        if field in numbers:
            validate_color(numbers[field], f"{path}.{field}", errors, file)


def validate_style(style_id: str, style: Any, path: str, errors: list, file: str):
    """Validate a single style definition."""
    if not isinstance(style, dict):
        errors.append(
            ValidationError(
                file, path, f"Style must be object, got {type(style).__name__}"
            )
        )
        return

    # Required: type
    if "type" not in style:
        errors.append(ValidationError(file, path, "Missing required field 'type'"))
        return

    style_type = style["type"]
    if style_type not in VALID_STYLE_TYPES:
        errors.append(
            ValidationError(
                file,
                f"{path}.type",
                f"Invalid style type '{style_type}'. Valid types: {VALID_STYLE_TYPES}",
            )
        )
        return

    # Optional common fields
    if "name" in style and not isinstance(style["name"], str):
        errors.append(ValidationError(file, f"{path}.name", "Name must be string"))

    if "description" in style and not isinstance(style["description"], str):
        errors.append(
            ValidationError(file, f"{path}.description", "Description must be string")
        )

    if "units" in style and not isinstance(style["units"], str):
        errors.append(ValidationError(file, f"{path}.units", "Units must be string"))

    if "transform" in style:
        validate_transform(style["transform"], f"{path}.transform", errors, file)

    if "range" in style:
        validate_range(style["range"], f"{path}.range", errors, file)

    if "legend" in style:
        validate_legend(style["legend"], f"{path}.legend", errors, file)

    # Type-specific validation
    if style_type in ("gradient", "filled_contour"):
        # Require stops for gradient/filled_contour
        if "stops" not in style:
            errors.append(
                ValidationError(
                    file, path, f"Style type '{style_type}' requires 'stops' array"
                )
            )
        elif not isinstance(style["stops"], list):
            errors.append(ValidationError(file, f"{path}.stops", "Stops must be array"))
        elif len(style["stops"]) < 2:
            errors.append(
                ValidationError(
                    file, f"{path}.stops", "Stops must have at least 2 entries"
                )
            )
        else:
            for i, stop in enumerate(style["stops"]):
                validate_stop(stop, i, f"{path}.stops", errors, file)

        if (
            "interpolation" in style
            and style["interpolation"] not in VALID_INTERPOLATION_TYPES
        ):
            errors.append(
                ValidationError(
                    file,
                    f"{path}.interpolation",
                    f"Invalid interpolation '{style['interpolation']}'. Valid: {VALID_INTERPOLATION_TYPES}",
                )
            )

        if (
            "out_of_range" in style
            and style["out_of_range"] not in VALID_OUT_OF_RANGE_TYPES
        ):
            errors.append(
                ValidationError(
                    file,
                    f"{path}.out_of_range",
                    f"Invalid out_of_range '{style['out_of_range']}'. Valid: {VALID_OUT_OF_RANGE_TYPES}",
                )
            )

    elif style_type == "contour":
        if "contour" in style:
            validate_contour(style["contour"], f"{path}.contour", errors, file)

    elif style_type in ("wind_barbs", "wind_arrows"):
        if "wind" in style:
            validate_wind(style["wind"], f"{path}.wind", errors, file)

        if "color_by_speed" in style:
            validate_color_by_speed(
                style["color_by_speed"], f"{path}.color_by_speed", errors, file
            )

    elif style_type == "numbers":
        if "numbers" in style:
            validate_numbers(style["numbers"], f"{path}.numbers", errors, file)


def validate_file(filepath: Path, verbose: bool = False) -> list:
    """Validate a single style JSON file."""
    errors = []
    filename = filepath.name

    if verbose:
        print(f"Validating {filename}...")

    # Read and parse JSON
    try:
        with open(filepath, "r") as f:
            data = json.load(f)
    except json.JSONDecodeError as e:
        errors.append(ValidationError(filename, "root", f"Invalid JSON: {e}"))
        return errors
    except Exception as e:
        errors.append(ValidationError(filename, "root", f"Could not read file: {e}"))
        return errors

    if not isinstance(data, dict):
        errors.append(
            ValidationError(
                filename, "root", f"Root must be object, got {type(data).__name__}"
            )
        )
        return errors

    # Required: version
    if "version" not in data:
        errors.append(
            ValidationError(filename, "root", "Missing required field 'version'")
        )
    elif data["version"] != "1.0":
        errors.append(
            ValidationError(
                filename,
                "version",
                f"Unknown version '{data['version']}'. Expected '1.0'",
            )
        )

    # Optional: metadata
    if "metadata" in data and not isinstance(data["metadata"], dict):
        errors.append(ValidationError(filename, "metadata", "Metadata must be object"))

    # Required: styles
    if "styles" not in data:
        errors.append(
            ValidationError(filename, "root", "Missing required field 'styles'")
        )
    elif not isinstance(data["styles"], dict):
        errors.append(
            ValidationError(
                filename,
                "styles",
                f"Styles must be object, got {type(data['styles']).__name__}",
            )
        )
    else:
        # Validate each style
        for style_id, style_def in data["styles"].items():
            # Skip comment keys
            if style_id.startswith("_"):
                continue
            validate_style(style_id, style_def, f"styles.{style_id}", errors, filename)

    return errors


def main():
    verbose = "--verbose" in sys.argv or "-v" in sys.argv

    # Find styles directory
    script_dir = Path(__file__).parent
    styles_dir = script_dir

    # Get all JSON files except schema.example.json
    json_files = sorted(
        [f for f in styles_dir.glob("*.json") if f.name != "schema.example.json"]
    )

    if not json_files:
        print("No style JSON files found!")
        sys.exit(1)

    print(f"Validating {len(json_files)} style files...\n")

    all_errors = []
    files_with_errors = 0

    for filepath in json_files:
        errors = validate_file(filepath, verbose)
        if errors:
            files_with_errors += 1
            all_errors.extend(errors)
            if verbose:
                for err in errors:
                    print(f"  ERROR: {err.path}: {err.message}")

    # Print summary
    print("-" * 60)
    if all_errors:
        print(f"\nFOUND {len(all_errors)} ERROR(S) in {files_with_errors} file(s):\n")
        for err in all_errors:
            print(f"  {err}")
        print()
        sys.exit(1)
    else:
        print(f"\nSUCCESS: All {len(json_files)} files are valid!")
        for f in json_files:
            print(f"  {f.name}")
        sys.exit(0)


if __name__ == "__main__":
    main()
