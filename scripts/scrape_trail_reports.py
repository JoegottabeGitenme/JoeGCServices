#!/usr/bin/env python3
"""
Phase 0 groundwork for the trail-conditions design (see
docs/trail-conditions-design.md, Rung 2): pull trail condition reports into
the `trail_reports` table so a future session can test whether the physics
feature stack (aspect, soil texture, canopy, frozen state) separates reported
condition classes at all, before any physics gets built.

This script does NOT call any API without explicit opt-in. Two independent
input modes, either or both may run in a single invocation:

  1. Trailforks API (gated behind TRAILFORKS_TOKEN)
     Trailforks' API terms for a derived commercial product are an open
     question noted in the design doc and have NOT been verified. This mode
     is a no-op unless TRAILFORKS_TOKEN is set -- verify terms before setting
     it in any scheduled/production context.

  2. Generic CSV import (--csv-file)
     For trail-org-provided condition exports (COMBA, Medicine Wheel, BMA,
     USFS/county open space) that don't come through an API at all. Expected
     columns: source_id,trail_ref,reported_at,condition_raw,condition_class,lon,lat
     (source_id/condition_class/lon/lat may be blank).

Both modes POST to the ingester's /ingest/trail-reports endpoint in batches,
matching the architecture convention used everywhere else in this codebase
(scrapers never touch Postgres directly -- see storm_events_runner.rs).

Usage:
  python3 scripts/scrape_trail_reports.py --csv-file reports.csv
  TRAILFORKS_TOKEN=xxx python3 scripts/scrape_trail_reports.py --trailforks --bbox -109.06,36.99,-102.04,41.00

Environment:
  INGESTER_URL       default http://localhost:8082
  TRAILFORKS_TOKEN    unset by default; required for --trailforks
"""

import argparse
import csv
import json
import os
import sys
import urllib.request
from datetime import datetime, timezone

INGESTER_URL = os.environ.get("INGESTER_URL", "http://localhost:8082")
BATCH_SIZE = 500

# Best-effort free-text -> normalized class mapping. Deliberately coarse;
# this is a label archive for a separability test, not a production
# classifier -- see design doc S6/S7 for why a validated classifier is a
# separate, later effort.
_CONDITION_KEYWORDS = [
    ("closed", "closed"),
    ("snow", "snow"),
    ("ice", "snow"),
    ("mud", "muddy"),
    ("soft", "muddy"),
    ("wet", "wet"),
    ("puddle", "wet"),
    ("dry", "dry"),
    ("firm", "dry"),
    ("good", "dry"),
    ("dusty", "dry"),
]


def normalize_condition(raw_text):
    """Best-effort keyword mapping from free text to a coarse class."""
    if not raw_text:
        return None
    lowered = raw_text.lower()
    for keyword, cls in _CONDITION_KEYWORDS:
        if keyword in lowered:
            return cls
    return "unknown"


def post_batch(reports, dry_run=False):
    if not reports:
        return 0
    if dry_run:
        print(f"[dry-run] would POST {len(reports)} reports")
        return len(reports)

    body = json.dumps({"reports": reports}).encode("utf-8")
    req = urllib.request.Request(
        f"{INGESTER_URL}/ingest/trail-reports",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        result = json.loads(resp.read().decode("utf-8"))
        if not result.get("success"):
            print(f"WARNING: ingest reported failure: {result.get('message')}", file=sys.stderr)
        return result.get("reports_ingested", 0)


def import_csv(path, dry_run=False):
    """Import a generic CSV export from a trail org.

    Expected columns (header row required):
      source_id,trail_ref,reported_at,condition_raw,condition_class,lon,lat

    reported_at must be ISO 8601 (e.g. 2026-09-15T14:30:00Z). condition_class
    is used verbatim if present; otherwise derived from condition_raw via
    normalize_condition().
    """
    total = 0
    batch = []
    with open(path, newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            reported_at = row.get("reported_at", "").strip()
            if not reported_at:
                print(f"Skipping row with no reported_at: {row}", file=sys.stderr)
                continue
            # Accept bare dates by assuming midnight UTC.
            if "T" not in reported_at:
                reported_at = f"{reported_at}T00:00:00Z"

            condition_raw = row.get("condition_raw", "").strip() or None
            condition_class = row.get("condition_class", "").strip() or normalize_condition(
                condition_raw
            )

            lon = row.get("lon", "").strip()
            lat = row.get("lat", "").strip()

            batch.append(
                {
                    "source": "csv_import",
                    "source_id": row.get("source_id", "").strip() or None,
                    "trail_ref": row.get("trail_ref", "").strip() or None,
                    "reported_at": reported_at,
                    "condition_raw": condition_raw,
                    "condition_class": condition_class,
                    "lon": float(lon) if lon else None,
                    "lat": float(lat) if lat else None,
                    "raw": row,
                }
            )
            if len(batch) >= BATCH_SIZE:
                total += post_batch(batch, dry_run)
                batch = []
    total += post_batch(batch, dry_run)
    print(f"CSV import complete: {total} reports ingested from {path}")
    return total


def fetch_trailforks(bbox, dry_run=False):
    """Fetch trail condition reports from the Trailforks API.

    Gated behind TRAILFORKS_TOKEN -- see the module docstring. This is a
    skeleton: Trailforks' actual condition-report endpoint shape has not
    been verified against a live account, and their ToS for a derived
    commercial product is an open question in the design doc. Fill in the
    real endpoint/response mapping once both are confirmed.
    """
    token = os.environ.get("TRAILFORKS_TOKEN")
    if not token:
        print(
            "TRAILFORKS_TOKEN not set -- skipping Trailforks fetch. "
            "See the module docstring: API terms for a derived commercial "
            "product have not been verified, this is opt-in only.",
        )
        return 0

    print(
        "TRAILFORKS_TOKEN is set, but the Trailforks API integration is a "
        "skeleton pending endpoint/ToS verification (see design doc S2/§11). "
        "Not fetching anything.",
        file=sys.stderr,
    )
    # Real implementation once verified:
    #   GET https://www.trailforks.com/api/1/<endpoint>?bbox=...&token=...
    #   map response rows -> the same report dict shape as import_csv()
    #   batch through post_batch()
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--csv-file", help="Path to a generic trail-org CSV export")
    parser.add_argument("--trailforks", action="store_true", help="Fetch from Trailforks (requires TRAILFORKS_TOKEN)")
    parser.add_argument("--bbox", help="min_lon,min_lat,max_lon,max_lat (Trailforks mode)")
    parser.add_argument("--dry-run", action="store_true", help="Print what would be sent, don't POST")
    args = parser.parse_args()

    if not args.csv_file and not args.trailforks:
        parser.error("Specify at least one of --csv-file or --trailforks")

    total = 0
    if args.csv_file:
        total += import_csv(args.csv_file, args.dry_run)
    if args.trailforks:
        bbox = tuple(float(x) for x in args.bbox.split(",")) if args.bbox else None
        total += fetch_trailforks(bbox, args.dry_run)

    print(f"Done. {total} total reports processed at {datetime.now(timezone.utc).isoformat()}")


if __name__ == "__main__":
    main()
