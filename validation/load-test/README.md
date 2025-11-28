# Weather WMS Load Testing Framework

A Rust-based load testing tool for measuring WMTS tile rendering performance.

## Quick Start

```bash
# From project root
./scripts/run_load_test.sh
```

This runs a quick smoke test (10 seconds, 1 concurrent request).

## Available Scenarios

| Scenario | Duration | Concurrency | Purpose |
|----------|----------|-------------|---------|
| `quick` | 10s | 1 | Fast smoke test |
| `cold_cache` | 60s | 10 | Test cache-miss performance |
| `warm_cache` | 60s | 10 | Test cache-hit performance |
| `stress` | 60s | 200 | High concurrency stress test |
| `layer_comparison` | 120s | 20 | Compare different layer types |

## Usage Examples

```bash
# Run different scenarios
./scripts/run_load_test.sh cold_cache
./scripts/run_load_test.sh stress --reset-cache

# Save results for tracking
./scripts/run_load_test.sh cold_cache --save --output json
./scripts/run_load_test.sh cold_cache --save --output csv

# Use custom scenario
./scripts/run_load_test.sh --scenario my_test.yaml
```

## Direct CLI Usage

```bash
# Build the tool
cargo build --package load-test --release

# Run a scenario
./target/release/load-test run --scenario scenarios/quick.yaml

# Quick ad-hoc test
./target/release/load-test quick --layer gfs_TMP --requests 100

# List scenarios
./target/release/load-test list
```

## Creating Custom Scenarios

Create a YAML file in `scenarios/`:

```yaml
name: my_test
description: Custom performance test
base_url: http://localhost:8080
duration_secs: 60
concurrency: 20
warmup_secs: 5  # Excluded from final stats
layers:
  - name: gfs_TMP
    style: temperature
    weight: 1.0  # Relative request frequency
  - name: hrrr_TMP
    style: temperature
    weight: 0.5
tile_selection:
  type: random
  zoom_range: [4, 8]
  bbox:  # Optional - constrain to region
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55
```

### Tile Selection Modes

**Random** - Generate random tiles within zoom range and optional bbox:
```yaml
tile_selection:
  type: random
  zoom_range: [3, 10]
  bbox:
    min_lon: -180
    min_lat: -90
    max_lon: 180
    max_lat: 90
```

**Sequential** - Iterate through all tiles in bbox (for cache warming):
```yaml
tile_selection:
  type: sequential
  zoom: 5
  bbox:
    min_lon: -130
    min_lat: 20
    max_lon: -60
    max_lat: 55
```

**Fixed** - Use specific tile coordinates:
```yaml
tile_selection:
  type: fixed
  tiles:
    - [5, 10, 12]  # [zoom, x, y]
    - [6, 20, 24]
```

**Pan Simulation** - Simulate user panning (future):
```yaml
tile_selection:
  type: pan_simulation
  start: [5, 10, 12]
  steps: 100
```

## Output Formats

### Table (default)
Human-readable console output with formatted statistics.

### JSON
Machine-readable format for automation:
```bash
./target/release/load-test run --scenario scenarios/quick.yaml --output json > results.json
```

### CSV
Append to CSV file for tracking performance over time:
```bash
./target/release/load-test run --scenario scenarios/quick.yaml --output csv >> results.csv
```

CSV includes: timestamp, scenario, duration, requests, RPS, latency percentiles, cache hit rate.

## Interpreting Results

Key metrics to watch:

**Latency Percentiles:**
- `p50` (median) - Typical request time
- `p90` - 90% of requests faster than this
- `p95` - 95% of requests faster than this
- `p99` - 99% of requests faster than this (catches outliers)

**Throughput:**
- `requests_per_second` - How many tiles/sec the system can serve
- `bytes_per_second` - Network throughput

**Cache Performance:**
- `cache_hit_rate` - Percentage served from Redis cache
- Higher is better for production workloads
- 0% for cold_cache tests (expected)
- Should be >80% in production

## Best Practices

1. **Always reset cache** before benchmark runs:
   ```bash
   ./scripts/reset_test_state.sh
   ```

2. **Run warmup period** to stabilize JIT compilation and async runtime

3. **Use consistent test conditions**:
   - Same dataset size
   - Same zoom levels
   - Same bbox regions

4. **Track results over time**:
   ```bash
   ./scripts/run_load_test.sh cold_cache --save --output csv
   ```
   Opens CSV in spreadsheet to identify performance regressions.

5. **Test different scenarios**:
   - Cold cache (cache misses - rendering performance)
   - Warm cache (cache hits - Redis performance)
   - Different layer types (gradient vs wind barbs vs isolines)
   - Different zoom levels (data volume varies)

## Architecture

```
validation/load-test/
├── src/
│   ├── config.rs      # YAML configuration loading
│   ├── generator.rs   # WMTS URL generation
│   ├── runner.rs      # HTTP request execution
│   ├── metrics.rs     # Statistics collection
│   └── report.rs      # Output formatting
├── scenarios/         # Test scenario definitions
└── results/           # Saved test results
```

## Development Status

**Implemented (Phase 2.1-2.3):**
- ✅ Project setup and dependencies
- ✅ Configuration system (YAML loading)
- ✅ Tile URL generator (lat/lon math, weighted layer selection)
- ✅ Basic CLI interface
- ✅ Metrics collection (HDR histograms)
- ✅ Report output (table, JSON, CSV)

**TODO (Phase 2.4-2.9):**
- ⏳ HTTP request runner with controlled concurrency
- ⏳ Progress bars during test execution
- ⏳ Rate limiting (requests/second cap)
- ⏳ Complete scenario implementations
- ⏳ Integration with CI/CD

## Contributing

When adding new scenarios:
1. Create YAML file in `scenarios/`
2. Test with `./scripts/run_load_test.sh --scenario your_scenario.yaml`
3. Document purpose and expected results

## Troubleshooting

**Service not responding:**
```bash
# Check if services are running
docker-compose ps

# Start services if needed
./scripts/start.sh
```

**Build errors:**
```bash
# Clean and rebuild
cargo clean
cargo build --package load-test
```

**Inconsistent results:**
- Reset cache between runs: `./scripts/reset_test_state.sh`
- Ensure no other load on the system
- Check Docker resource limits
