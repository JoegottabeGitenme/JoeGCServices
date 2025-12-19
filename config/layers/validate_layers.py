#!/usr/bin/env python3
"""
Validate WMS layer configuration files.

Checks:
- YAML syntax validity
- No duplicate layer IDs across all files
- Required fields present (id, parameter, title, style_file, units.native)
- Referenced style files exist
- Layer IDs follow naming convention ({model}_{parameter})
- Each layer has at least one level with a default
"""

import sys
import os
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:
    print("ERROR: PyYAML not installed. Run: pip install pyyaml")
    sys.exit(1)


REQUIRED_LAYER_FIELDS = ["id", "parameter", "title", "style_file"]
REQUIRED_MODEL_FIELDS = ["model", "display_name", "layers"]


def load_yaml_file(filepath: Path) -> dict | None:
    """Load and parse a YAML file, returning None on error."""
    try:
        with open(filepath, "r") as f:
            return yaml.safe_load(f)
    except yaml.YAMLError as e:
        print(f"  ERROR: Invalid YAML syntax in {filepath.name}")
        print(f"         {e}")
        return None
    except Exception as e:
        print(f"  ERROR: Could not read {filepath.name}: {e}")
        return None


def validate_layer(
    layer: dict[str, Any],
    model: str,
    style_dir: Path,
    errors: list[str],
    warnings: list[str],
) -> str | None:
    """Validate a single layer configuration. Returns layer ID if valid."""
    layer_id = layer.get("id", "<missing>")

    # Check required fields
    for field in REQUIRED_LAYER_FIELDS:
        if field not in layer:
            errors.append(f"Layer '{layer_id}': missing required field '{field}'")

    # Check units.native (not required for composite layers)
    is_composite = layer.get("composite", False)
    units = layer.get("units", {})
    if units and not isinstance(units, dict):
        errors.append(f"Layer '{layer_id}': 'units' must be an object")
    elif not is_composite:
        if not units:
            errors.append(f"Layer '{layer_id}': missing required field 'units'")
        elif "native" not in units:
            errors.append(f"Layer '{layer_id}': missing required field 'units.native'")

    # Check layer ID naming convention
    if "id" in layer and "parameter" in layer:
        expected_prefix = f"{model}_"
        if not layer_id.startswith(expected_prefix):
            warnings.append(
                f"Layer '{layer_id}': ID should start with '{expected_prefix}'"
            )

    # Check style file exists
    style_file = layer.get("style_file")
    if style_file:
        style_path = style_dir / style_file
        if not style_path.exists():
            errors.append(
                f"Layer '{layer_id}': style file '{style_file}' not found in config/styles/"
            )

    # Check levels have a default
    levels = layer.get("levels", [])
    if levels:
        has_default = any(
            isinstance(lv, dict) and lv.get("default", False) for lv in levels
        )
        if not has_default:
            warnings.append(f"Layer '{layer_id}': no default level specified")

    # Check composite layers have requires field
    if layer.get("composite") and not layer.get("requires"):
        errors.append(f"Layer '{layer_id}': composite layer must have 'requires' field")

    return layer_id if "id" in layer else None


def validate_file(
    filepath: Path, style_dir: Path, all_layer_ids: dict[str, str]
) -> tuple[int, int]:
    """Validate a single layer config file. Returns (error_count, warning_count)."""
    errors: list[str] = []
    warnings: list[str] = []

    print(f"\nValidating {filepath.name}...")

    data = load_yaml_file(filepath)
    if data is None:
        return 1, 0

    # Check required model fields
    for field in REQUIRED_MODEL_FIELDS:
        if field not in data:
            errors.append(f"Missing required field '{field}'")

    model = data.get("model", "unknown")
    layers = data.get("layers", [])

    if not isinstance(layers, list):
        errors.append("'layers' must be a list")
        layers = []

    # Validate each layer
    for layer in layers:
        if not isinstance(layer, dict):
            errors.append(f"Layer entry must be an object, got: {type(layer).__name__}")
            continue

        layer_id = validate_layer(layer, model, style_dir, errors, warnings)

        if layer_id:
            # Check for duplicate IDs
            if layer_id in all_layer_ids:
                errors.append(
                    f"Duplicate layer ID '{layer_id}' (also in {all_layer_ids[layer_id]})"
                )
            else:
                all_layer_ids[layer_id] = filepath.name

    # Print results for this file
    if errors:
        for err in errors:
            print(f"  ERROR: {err}")
    if warnings:
        for warn in warnings:
            print(f"  WARNING: {warn}")
    if not errors and not warnings:
        print(f"  OK ({len(layers)} layers)")

    return len(errors), len(warnings)


def main():
    # Determine paths
    script_dir = Path(__file__).parent
    style_dir = script_dir.parent / "styles"

    # Find all layer YAML files (exclude README)
    layer_files = sorted(script_dir.glob("*.yaml"))

    if not layer_files:
        print("ERROR: No .yaml files found in config/layers/")
        sys.exit(1)

    print(f"Layer Configuration Validator")
    print(f"=" * 50)
    print(f"Found {len(layer_files)} layer config file(s)")
    print(f"Style directory: {style_dir}")

    # Track all layer IDs across files
    all_layer_ids: dict[str, str] = {}
    total_errors = 0
    total_warnings = 0

    for filepath in layer_files:
        errors, warnings = validate_file(filepath, style_dir, all_layer_ids)
        total_errors += errors
        total_warnings += warnings

    # Summary
    print(f"\n{'=' * 50}")
    print(f"Summary: {len(all_layer_ids)} total layers across {len(layer_files)} files")

    if total_errors:
        print(f"\nFAILED: {total_errors} error(s), {total_warnings} warning(s)")
        sys.exit(1)
    elif total_warnings:
        print(f"\nPASSED with {total_warnings} warning(s)")
        sys.exit(0)
    else:
        print(f"\nPASSED: All validations successful")
        sys.exit(0)


if __name__ == "__main__":
    main()
