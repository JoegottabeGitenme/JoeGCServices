#!/usr/bin/env python3
"""
EDR Product Dump

Queries every parameter/level/time combination advertised by the EDR API
and dumps the results to JSON files for manual verification.

Usage:
    python edr_product_dump.py [--endpoint URL] [--output DIR] [--query-type TYPE]

Options:
    --endpoint URL      EDR API endpoint (default: http://localhost:8083/edr)
    --output DIR        Output directory (default: ./edr-dump-TIMESTAMP)
    --query-type TYPE   Query type: position, area, locations (default: position)
    --location ID       Location ID for locations queries (default: first available)
    --limit N           Max queries per collection (default: unlimited)
    --format FMT        Output format: covjson, geojson, both (default: both)
"""

import argparse
import json
import os
import sys
from datetime import datetime
from pathlib import Path
from typing import Any, Optional
from urllib.parse import quote, urljoin

try:
    import requests
except ImportError:
    print("ERROR: requests library required. Install with: pip install requests")
    sys.exit(1)


class EDRProductDumper:
    def __init__(
        self,
        endpoint: str,
        output_dir: str,
        query_type: str = "position",
        location_id: Optional[str] = None,
        limit: Optional[int] = None,
        output_format: str = "both",
    ):
        self.endpoint = endpoint.rstrip("/")
        self.output_dir = Path(output_dir)
        self.query_type = query_type
        self.location_id = location_id
        self.limit = limit
        self.output_format = output_format

        # Test coordinates (center of CONUS)
        self.test_point = {"lon": -100, "lat": 40}

        # Stats
        self.stats = {
            "total": 0,
            "success": 0,
            "empty": 0,
            "failed": 0,
            "errors": [],
        }

        # Collected results for summary
        self.results = []

    def run(self):
        """Main entry point."""
        print("=" * 60)
        print("EDR Product Dump")
        print("=" * 60)
        print(f"Endpoint:    {self.endpoint}")
        print(f"Output:      {self.output_dir}")
        print(f"Query type:  {self.query_type}")
        print(f"Format:      {self.output_format}")
        print()

        # Create output directories
        self.output_dir.mkdir(parents=True, exist_ok=True)
        (self.output_dir / "covjson").mkdir(exist_ok=True)
        (self.output_dir / "geojson").mkdir(exist_ok=True)
        (self.output_dir / "metadata").mkdir(exist_ok=True)

        # Fetch collections
        collections = self.fetch_collections()
        if not collections:
            print("ERROR: No collections found")
            return

        print(f"Found {len(collections)} collections")
        print()

        # Process each collection
        for coll in collections:
            self.process_collection(coll)

        # Write summary
        self.write_summary()
        self.write_index_html()

        # Print final stats
        print()
        print("=" * 60)
        print("Summary")
        print("=" * 60)
        print(f"Total queries:  {self.stats['total']}")
        print(f"Success:        {self.stats['success']}")
        print(f"Empty:          {self.stats['empty']}")
        print(f"Failed:         {self.stats['failed']}")
        print()
        print(f"Results saved to: {self.output_dir}")
        print(f"View results: python3 -m http.server -d {self.output_dir} 8000")

    def fetch_collections(self) -> list:
        """Fetch list of collections from API."""
        try:
            resp = requests.get(f"{self.endpoint}/collections", timeout=30)
            resp.raise_for_status()
            data = resp.json()

            # Save to metadata
            with open(self.output_dir / "metadata" / "collections.json", "w") as f:
                json.dump(data, f, indent=2)

            return data.get("collections", [])
        except Exception as e:
            print(f"ERROR fetching collections: {e}")
            return []

    def process_collection(self, coll: dict):
        """Process a single collection."""
        coll_id = coll.get("id", "unknown")
        print()
        print(f"Processing: {coll_id}")
        print("-" * 40)

        # Fetch full collection details
        try:
            resp = requests.get(f"{self.endpoint}/collections/{coll_id}", timeout=30)
            resp.raise_for_status()
            coll_detail = resp.json()

            # Save metadata
            with open(self.output_dir / "metadata" / f"{coll_id}.json", "w") as f:
                json.dump(coll_detail, f, indent=2)
        except Exception as e:
            print(f"  ERROR fetching collection details: {e}")
            return

        # Extract parameters
        params = list(coll_detail.get("parameter_names", {}).keys())
        if not params:
            print("  No parameters found, skipping")
            return

        # Extract levels
        levels = []
        vertical = coll_detail.get("extent", {}).get("vertical", {})
        for interval in vertical.get("interval", []):
            if interval and interval[0] is not None:
                levels.append(interval[0])
        levels = sorted(set(levels))

        # Extract latest time
        times = coll_detail.get("extent", {}).get("temporal", {}).get("values", [])
        latest_time = times[0] if times else None

        print(f"  Parameters: {len(params)}")
        print(f"  Levels: {len(levels) if levels else 'none'}")
        print(f"  Latest time: {latest_time or 'none'}")

        # Create output directories
        (self.output_dir / "covjson" / coll_id).mkdir(exist_ok=True)
        (self.output_dir / "geojson" / coll_id).mkdir(exist_ok=True)

        # If using locations query, fetch available locations
        location_id = self.location_id
        if self.query_type == "locations" and not location_id:
            location_id = self.fetch_first_location(coll_id)
            if not location_id:
                print("  No locations available, skipping")
                return

        # Query each parameter/level combination
        query_count = 0
        for param in params:
            level_list = levels if levels else [None]

            for level in level_list:
                if self.limit and query_count >= self.limit:
                    print(f"  Reached limit of {self.limit} queries")
                    return

                self.query_product(coll_id, param, level, latest_time, location_id)
                query_count += 1

    def fetch_first_location(self, coll_id: str) -> Optional[str]:
        """Fetch the first available location for a collection."""
        try:
            resp = requests.get(
                f"{self.endpoint}/collections/{coll_id}/locations", timeout=30
            )
            resp.raise_for_status()
            data = resp.json()
            features = data.get("features", [])
            if features:
                # Extract location ID from URI if needed
                raw_id = features[0].get("id", "")
                if "/locations/" in raw_id:
                    return raw_id.split("/locations/")[-1]
                return raw_id
        except Exception as e:
            print(f"  Warning: Could not fetch locations: {e}")
        return None

    def query_product(
        self,
        coll_id: str,
        param: str,
        level: Optional[float],
        time: Optional[str],
        location_id: Optional[str] = None,
    ):
        """Query a single product and save results."""
        self.stats["total"] += 1

        # Build filename
        if level is not None:
            filename = f"{param}_z{level}"
        else:
            filename = param

        # Build query URL
        if self.query_type == "locations" and location_id:
            base_url = f"{self.endpoint}/collections/{coll_id}/locations/{location_id}"
        elif self.query_type == "area":
            # 1x1 degree box
            lon, lat = self.test_point["lon"], self.test_point["lat"]
            polygon = f"POLYGON(({lon - 0.5} {lat - 0.5},{lon + 0.5} {lat - 0.5},{lon + 0.5} {lat + 0.5},{lon - 0.5} {lat + 0.5},{lon - 0.5} {lat - 0.5}))"
            base_url = f"{self.endpoint}/collections/{coll_id}/area"
        else:
            coords = f"POINT({self.test_point['lon']} {self.test_point['lat']})"
            base_url = f"{self.endpoint}/collections/{coll_id}/position"

        # Build query params
        query_params = [f"parameter-name={param}"]

        if self.query_type == "position":
            coords = f"POINT({self.test_point['lon']} {self.test_point['lat']})"
            query_params.append(f"coords={quote(coords)}")
        elif self.query_type == "area":
            lon, lat = self.test_point["lon"], self.test_point["lat"]
            polygon = f"POLYGON(({lon - 0.5} {lat - 0.5},{lon + 0.5} {lat - 0.5},{lon + 0.5} {lat + 0.5},{lon - 0.5} {lat + 0.5},{lon - 0.5} {lat - 0.5}))"
            query_params.append(f"coords={quote(polygon)}")

        if level is not None:
            query_params.append(f"z={level}")

        if time:
            query_params.append(f"datetime={quote(time)}")

        # Print progress
        level_str = f" @ z={level}" if level is not None else ""
        print(f"  {param}{level_str}...", end=" ", flush=True)

        # Query CoverageJSON
        result = {
            "collection": coll_id,
            "parameter": param,
            "level": level,
            "time": time,
            "status": "unknown",
        }

        formats_to_query = []
        if self.output_format in ("covjson", "both"):
            formats_to_query.append(("covjson", ""))
        if self.output_format in ("geojson", "both"):
            formats_to_query.append(("geojson", "&f=geojson"))

        for fmt, fmt_param in formats_to_query:
            url = f"{base_url}?{'&'.join(query_params)}{fmt_param}"
            result[f"{fmt}_url"] = url

            try:
                resp = requests.get(url, timeout=60)
                output_file = self.output_dir / fmt / coll_id / f"{filename}.json"

                if resp.status_code == 200:
                    data = resp.json()
                    with open(output_file, "w") as f:
                        json.dump(data, f, indent=2)

                    # Check for actual data
                    if fmt == "covjson":
                        values = data.get("ranges", {}).get(param, {}).get("values", [])
                        non_null = len([v for v in values if v is not None])
                        result["value_count"] = len(values)
                        result["non_null_count"] = non_null

                        if non_null > 0:
                            result["status"] = "success"
                        else:
                            result["status"] = "empty"
                    else:
                        # GeoJSON
                        features = data.get("features", [])
                        result["feature_count"] = len(features)
                else:
                    result["status"] = "failed"
                    result["error"] = f"HTTP {resp.status_code}"
                    with open(output_file, "w") as f:
                        f.write(resp.text)

            except Exception as e:
                result["status"] = "failed"
                result["error"] = str(e)
                self.stats["errors"].append(
                    {"collection": coll_id, "param": param, "error": str(e)}
                )

        # Update stats and print result
        if result["status"] == "success":
            self.stats["success"] += 1
            print(f"OK ({result.get('non_null_count', '?')} values)")
        elif result["status"] == "empty":
            self.stats["empty"] += 1
            print("EMPTY")
        else:
            self.stats["failed"] += 1
            print(f"FAILED: {result.get('error', 'unknown')}")

        self.results.append(result)

    def write_summary(self):
        """Write summary files."""
        # Text summary
        with open(self.output_dir / "summary.txt", "w") as f:
            f.write("EDR Product Dump Summary\n")
            f.write("=" * 40 + "\n")
            f.write(f"Endpoint: {self.endpoint}\n")
            f.write(f"Timestamp: {datetime.now().isoformat()}\n")
            f.write(f"Query type: {self.query_type}\n")
            f.write("\n")
            f.write(f"Total queries:  {self.stats['total']}\n")
            f.write(f"Success:        {self.stats['success']}\n")
            f.write(f"Empty:          {self.stats['empty']}\n")
            f.write(f"Failed:         {self.stats['failed']}\n")

            if self.stats["errors"]:
                f.write("\nErrors:\n")
                for err in self.stats["errors"][:20]:  # First 20 errors
                    f.write(f"  {err['collection']}/{err['param']}: {err['error']}\n")

        # JSON results
        with open(self.output_dir / "results.json", "w") as f:
            json.dump(
                {
                    "endpoint": self.endpoint,
                    "timestamp": datetime.now().isoformat(),
                    "query_type": self.query_type,
                    "stats": self.stats,
                    "results": self.results,
                },
                f,
                indent=2,
            )

    def write_index_html(self):
        """Write an HTML index for viewing results."""
        html = """<!DOCTYPE html>
<html>
<head>
    <title>EDR Product Dump Results</title>
    <style>
        * { box-sizing: border-box; }
        body { 
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, monospace;
            margin: 0; padding: 20px;
            background: #0f172a; color: #e2e8f0;
        }
        h1 { color: #38bdf8; margin-bottom: 5px; }
        h2 { color: #7dd3fc; margin-top: 30px; }
        .stats { 
            display: flex; gap: 20px; margin: 20px 0;
            flex-wrap: wrap;
        }
        .stat { 
            padding: 15px 25px; border-radius: 8px;
            background: #1e293b;
        }
        .stat-value { font-size: 2em; font-weight: bold; }
        .stat-label { color: #94a3b8; font-size: 0.9em; }
        .stat.success .stat-value { color: #4ade80; }
        .stat.empty .stat-value { color: #fbbf24; }
        .stat.failed .stat-value { color: #f87171; }
        
        .collection { 
            margin: 20px 0; padding: 20px;
            background: #1e293b; border-radius: 8px;
        }
        .collection h3 { margin-top: 0; color: #38bdf8; }
        
        .results-table { width: 100%; border-collapse: collapse; margin-top: 15px; }
        .results-table th, .results-table td { 
            padding: 8px 12px; text-align: left;
            border-bottom: 1px solid #334155;
        }
        .results-table th { color: #94a3b8; font-weight: normal; }
        .results-table tr:hover { background: #334155; }
        
        .status { padding: 2px 8px; border-radius: 4px; font-size: 0.85em; }
        .status.success { background: #166534; color: #4ade80; }
        .status.empty { background: #854d0e; color: #fbbf24; }
        .status.failed { background: #991b1b; color: #f87171; }
        
        a { color: #38bdf8; text-decoration: none; }
        a:hover { text-decoration: underline; }
        
        .viewer { 
            position: fixed; right: 20px; top: 20px;
            width: 45%; height: calc(100vh - 40px);
            background: #1e293b; border-radius: 8px;
            display: flex; flex-direction: column;
        }
        .viewer-header { 
            padding: 15px; border-bottom: 1px solid #334155;
            display: flex; justify-content: space-between; align-items: center;
        }
        .viewer-content { 
            flex: 1; overflow: auto; padding: 15px;
        }
        .viewer pre { 
            margin: 0; white-space: pre-wrap; 
            font-size: 12px; line-height: 1.4;
        }
        
        .main-content { width: 50%; }
        
        @media (max-width: 1200px) {
            .viewer { position: static; width: 100%; height: 500px; margin-top: 20px; }
            .main-content { width: 100%; }
        }
    </style>
</head>
<body>
    <div class="main-content">
        <h1>EDR Product Dump Results</h1>
        <p id="meta"></p>
        
        <div class="stats">
            <div class="stat"><div class="stat-value" id="total">-</div><div class="stat-label">Total</div></div>
            <div class="stat success"><div class="stat-value" id="success">-</div><div class="stat-label">Success</div></div>
            <div class="stat empty"><div class="stat-value" id="empty">-</div><div class="stat-label">Empty</div></div>
            <div class="stat failed"><div class="stat-value" id="failed">-</div><div class="stat-label">Failed</div></div>
        </div>
        
        <div id="collections"></div>
    </div>
    
    <div class="viewer">
        <div class="viewer-header">
            <span id="viewer-title">JSON Viewer</span>
            <span id="viewer-meta"></span>
        </div>
        <div class="viewer-content">
            <pre id="viewer-json">Click a result to view JSON</pre>
        </div>
    </div>
    
    <script>
        let resultsData = null;
        
        async function loadFile(path, title) {
            document.getElementById('viewer-title').textContent = title;
            document.getElementById('viewer-meta').textContent = 'Loading...';
            
            try {
                const resp = await fetch(path);
                if (!resp.ok) {
                    document.getElementById('viewer-json').textContent = `HTTP Error: ${resp.status} ${resp.statusText}`;
                    document.getElementById('viewer-meta').textContent = 'Error';
                    return;
                }
                
                const text = await resp.text();
                
                // Try to parse as JSON
                try {
                    const data = JSON.parse(text);
                    document.getElementById('viewer-json').textContent = JSON.stringify(data, null, 2);
                    
                    // Show some metadata
                    let meta = '';
                    if (data.type === 'Coverage') {
                        const param = Object.keys(data.ranges || {})[0];
                        if (param) {
                            const values = data.ranges[param].values || [];
                            const nonNull = values.filter(v => v !== null).length;
                            meta = `${nonNull}/${values.length} values`;
                        }
                    } else if (data.type === 'FeatureCollection') {
                        meta = `${(data.features || []).length} features`;
                    } else if (data.type === 'CoverageCollection') {
                        meta = `${(data.coverages || []).length} coverages`;
                    }
                    document.getElementById('viewer-meta').textContent = meta;
                } catch (parseErr) {
                    // Not valid JSON - show raw text (might be an error response)
                    document.getElementById('viewer-json').textContent = text || '(empty file)';
                    document.getElementById('viewer-meta').textContent = 'Not JSON';
                }
            } catch (e) {
                document.getElementById('viewer-json').textContent = 'Error loading file: ' + e.message;
                document.getElementById('viewer-meta').textContent = 'Error';
            }
        }
        
        async function init() {
            try {
                const resp = await fetch('results.json');
                resultsData = await resp.json();
                
                // Update stats
                document.getElementById('total').textContent = resultsData.stats.total;
                document.getElementById('success').textContent = resultsData.stats.success;
                document.getElementById('empty').textContent = resultsData.stats.empty;
                document.getElementById('failed').textContent = resultsData.stats.failed;
                document.getElementById('meta').textContent = 
                    `Endpoint: ${resultsData.endpoint} | Query: ${resultsData.query_type} | Time: ${resultsData.timestamp}`;
                
                // Group results by collection
                const byCollection = {};
                for (const r of resultsData.results) {
                    if (!byCollection[r.collection]) {
                        byCollection[r.collection] = [];
                    }
                    byCollection[r.collection].push(r);
                }
                
                // Render collections
                let html = '';
                for (const [collId, results] of Object.entries(byCollection)) {
                    html += `<div class="collection">`;
                    html += `<h3>${collId}</h3>`;
                    html += `<table class="results-table">`;
                    html += `<tr><th>Parameter</th><th>Level</th><th>Status</th><th>Values</th><th>Files</th></tr>`;
                    
                    for (const r of results) {
                        const levelStr = r.level !== null ? r.level : '-';
                        const valuesStr = r.non_null_count !== undefined 
                            ? `${r.non_null_count}/${r.value_count}` 
                            : '-';
                        const filename = r.level !== null ? `${r.parameter}_z${r.level}` : r.parameter;
                        
                        html += `<tr>`;
                        html += `<td>${r.parameter}</td>`;
                        html += `<td>${levelStr}</td>`;
                        html += `<td><span class="status ${r.status}">${r.status}</span></td>`;
                        html += `<td>${valuesStr}</td>`;
                        html += `<td>`;
                        html += `<a href="#" onclick="loadFile('covjson/${collId}/${filename}.json', '${r.parameter} CovJSON'); return false;">covjson</a> `;
                        html += `<a href="#" onclick="loadFile('geojson/${collId}/${filename}.json', '${r.parameter} GeoJSON'); return false;">geojson</a>`;
                        html += `</td>`;
                        html += `</tr>`;
                    }
                    
                    html += `</table>`;
                    html += `</div>`;
                }
                
                document.getElementById('collections').innerHTML = html;
                
            } catch (e) {
                document.getElementById('collections').innerHTML = '<p>Error loading results: ' + e.message + '</p>';
            }
        }
        
        init();
    </script>
</body>
</html>"""
        with open(self.output_dir / "index.html", "w") as f:
            f.write(html)


def main():
    parser = argparse.ArgumentParser(description="EDR Product Dump")
    parser.add_argument(
        "--endpoint",
        default="http://localhost:8083/edr",
        help="EDR API endpoint",
    )
    parser.add_argument(
        "--output",
        default=f"./edr-dump-{datetime.now().strftime('%Y%m%d-%H%M%S')}",
        help="Output directory",
    )
    parser.add_argument(
        "--query-type",
        choices=["position", "area", "locations"],
        default="position",
        help="Query type to use",
    )
    parser.add_argument(
        "--location",
        help="Location ID for locations queries",
    )
    parser.add_argument(
        "--limit",
        type=int,
        help="Max queries per collection",
    )
    parser.add_argument(
        "--format",
        choices=["covjson", "geojson", "both"],
        default="both",
        help="Output format",
    )

    args = parser.parse_args()

    dumper = EDRProductDumper(
        endpoint=args.endpoint,
        output_dir=args.output,
        query_type=args.query_type,
        location_id=args.location,
        limit=args.limit,
        output_format=args.format,
    )
    dumper.run()


if __name__ == "__main__":
    main()
