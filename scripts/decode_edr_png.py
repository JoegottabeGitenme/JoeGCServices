#!/usr/bin/env python3
"""
Decode and visualize EDR 16-bit PNG data.

This script fetches a PNG from the EDR API, decodes the 16-bit values,
and optionally displays or saves a visualization.

Usage:
    python scripts/decode_edr_png.py [options]

Examples:
    # Fetch and decode, show stats
    python scripts/decode_edr_png.py

    # Save visualization
    python scripts/decode_edr_png.py --save output.png

    # Custom URL
    python scripts/decode_edr_png.py --url "http://localhost:8083/edr/collections/hrrr-surface/area?..."

    # Decode existing file
    python scripts/decode_edr_png.py --file /tmp/edr-png-test/test1_basic.png --min 250 --max 310
"""

import argparse
import sys
from io import BytesIO

try:
    import numpy as np
    from PIL import Image

    HAS_DEPS = True
except ImportError:
    HAS_DEPS = False

try:
    import matplotlib.pyplot as plt
    import matplotlib.colors as mcolors

    HAS_MATPLOTLIB = True
except ImportError:
    HAS_MATPLOTLIB = False

try:
    import requests

    HAS_REQUESTS = True
except ImportError:
    HAS_REQUESTS = False


def decode_16bit_png(img_array):
    """
    Decode 16-bit values from RGBA PNG.

    Returns:
        values: Normalized values (0-1 range)
        valid_mask: Boolean mask of valid pixels
    """
    # R channel = high byte, G channel = low byte
    r = img_array[:, :, 0].astype(np.uint16)
    g = img_array[:, :, 1].astype(np.uint16)
    a = img_array[:, :, 3]

    # Combine to 16-bit value
    uint16_values = (r << 8) | g

    # Normalize to 0-1
    normalized = uint16_values.astype(np.float32) / 65535.0

    # Valid mask from alpha channel
    valid_mask = a > 127

    return normalized, valid_mask


def denormalize(normalized: np.ndarray, min_val: float, max_val: float) -> np.ndarray:
    """Convert normalized values back to physical units."""
    return normalized * (max_val - min_val) + min_val


def fetch_png(url: str) -> tuple[bytes, dict]:
    """Fetch PNG from URL and return bytes + headers."""
    if not HAS_REQUESTS:
        print("Error: requests library required. Install with: pip install requests")
        sys.exit(1)

    response = requests.get(url)
    response.raise_for_status()

    # Extract EDR headers
    headers = {}
    for key in [
        "x-edr-parameter",
        "x-edr-units",
        "x-edr-min",
        "x-edr-max",
        "x-edr-width",
        "x-edr-height",
        "x-edr-bbox",
    ]:
        if key in response.headers:
            headers[key] = response.headers[key]

    return response.content, headers


def main():
    parser = argparse.ArgumentParser(description="Decode EDR 16-bit PNG data")
    parser.add_argument("--url", help="EDR PNG URL to fetch")
    parser.add_argument("--file", help="Local PNG file to decode")
    parser.add_argument(
        "--min", type=float, help="Min value for decoding (from X-EDR-Min header)"
    )
    parser.add_argument(
        "--max", type=float, help="Max value for decoding (from X-EDR-Max header)"
    )
    parser.add_argument("--save", help="Save visualization to file")
    parser.add_argument(
        "--show", action="store_true", help="Show visualization (requires display)"
    )
    parser.add_argument(
        "--colormap", default="viridis", help="Matplotlib colormap (default: viridis)"
    )
    args = parser.parse_args()

    if not HAS_DEPS:
        print("Error: numpy and Pillow required. Install with:")
        print("  pip install numpy Pillow")
        sys.exit(1)

    # Default test URL
    if not args.url and not args.file:
        args.url = (
            "http://localhost:8083/edr/collections/hrrr-surface/area?"
            "coords=POLYGON((-100 35,-98 35,-98 37,-100 37,-100 35))&"
            "parameter-name=TMP&f=png"
        )
        print(f"Using default URL: {args.url[:80]}...")

    # Load PNG
    headers = {}
    if args.file:
        print(f"Loading from file: {args.file}")
        with open(args.file, "rb") as f:
            png_bytes = f.read()
    else:
        print(f"Fetching from URL...")
        png_bytes, headers = fetch_png(args.url)

    # Parse image
    img = Image.open(BytesIO(png_bytes))
    img_array = np.array(img)

    print(f"\nPNG Info:")
    print(f"  Mode: {img.mode}")
    print(f"  Size: {img.size[0]}x{img.size[1]}")
    print(f"  Bytes: {len(png_bytes):,}")

    if headers:
        print(f"\nEDR Headers:")
        for k, v in headers.items():
            print(f"  {k}: {v}")

    # Decode 16-bit values
    if img.mode != "RGBA":
        print(f"\nWarning: Expected RGBA, got {img.mode}")
        if img.mode == "RGB":
            # Add alpha channel
            img = img.convert("RGBA")
            img_array = np.array(img)

    normalized, valid_mask = decode_16bit_png(img_array)

    # Get min/max for denormalization
    min_val = args.min
    max_val = args.max
    if min_val is None and "x-edr-min" in headers:
        min_val = float(headers["x-edr-min"])
    if max_val is None and "x-edr-max" in headers:
        max_val = float(headers["x-edr-max"])

    print(f"\nDecoded Data:")
    print(
        f"  Valid pixels: {valid_mask.sum():,} / {valid_mask.size:,} ({100 * valid_mask.sum() / valid_mask.size:.1f}%)"
    )
    print(
        f"  Normalized range: {normalized[valid_mask].min():.4f} - {normalized[valid_mask].max():.4f}"
    )

    if min_val is not None and max_val is not None:
        physical = denormalize(normalized, min_val, max_val)
        valid_physical = physical[valid_mask]
        units = headers.get("x-edr-units", "")
        print(
            f"  Physical range: {valid_physical.min():.2f} - {valid_physical.max():.2f} {units}"
        )
        print(f"  Physical mean: {valid_physical.mean():.2f} {units}")
    else:
        physical = normalized
        print("  (No min/max provided, showing normalized values)")

    # Visualization
    if args.save or args.show:
        if not HAS_MATPLOTLIB:
            print("\nError: matplotlib required for visualization. Install with:")
            print("  pip install matplotlib")
            sys.exit(1)

        # Create masked array for visualization
        masked_data = np.ma.masked_where(~valid_mask, physical)

        fig, axes = plt.subplots(1, 2, figsize=(12, 5))

        # Data visualization
        ax1 = axes[0]
        im = ax1.imshow(masked_data, cmap=args.colormap, origin="upper")
        ax1.set_title(
            f"{headers.get('x-edr-parameter', 'Data')} ({headers.get('x-edr-units', '')})"
        )
        plt.colorbar(im, ax=ax1, label=headers.get("x-edr-units", "value"))

        # Valid mask visualization
        ax2 = axes[1]
        ax2.imshow(valid_mask, cmap="gray", origin="upper")
        ax2.set_title("Valid Data Mask (white=valid)")

        plt.tight_layout()

        if args.save:
            plt.savefig(args.save, dpi=150)
            print(f"\nSaved visualization to: {args.save}")

        if args.show:
            plt.show()

    # Print sample GLSL decoding code
    print(f"\n--- GLSL Decoding Example ---")
    print("""
// In fragment shader:
vec4 texel = texture2D(uDataTexture, vTexCoord);
float encoded = texel.r * 255.0 * 256.0 + texel.g * 255.0;
float normalized = encoded / 65535.0;
float value = normalized * (uMaxValue - uMinValue) + uMinValue;
bool valid = texel.a > 0.5;
""")

    if min_val is not None and max_val is not None:
        print(f"// Uniforms for this data:")
        print(f"uniform float uMinValue = {min_val};")
        print(f"uniform float uMaxValue = {max_val};")


if __name__ == "__main__":
    main()
