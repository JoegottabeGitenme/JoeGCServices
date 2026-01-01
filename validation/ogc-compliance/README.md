# OGC Compliance Testing

Run official OGC compliance tests for WMS 1.3.0, WMTS 1.0.0, and EDR 1.0 APIs using the OGC Executable Test Suites (ETS).

## Quick Start

```bash
# Run all compliance tests
./run_all_compliance.sh

# Or run individual test suites
./run_wms_compliance.sh
./run_wmts_compliance.sh
./run_edr_compliance.sh
```

## Prerequisites

- **Docker** (for WMS and WMTS tests)
- **Java 17+** (for EDR tests only)
- Services running:
  - WMS/WMTS on port 8080 (default)
  - EDR API on port 8083 (default)

## Test Suites

| Service | OGC Standard | ETS Version | Test Method |
|---------|--------------|-------------|-------------|
| WMS | 1.3.0 | latest | Docker + TEAM Engine |
| WMTS | 1.0.0 | latest | Docker + TEAM Engine |
| EDR | 1.0 | 1.3 | Standalone JAR (TestNG) |

**Note:** WMS and WMTS tests use Docker containers running official OGC TEAM Engine images. EDR tests run standalone via Java.

## Scripts

### Run All Tests

```bash
# Run all three test suites
./run_all_compliance.sh

# Skip specific services
./run_all_compliance.sh --skip-edr
./run_all_compliance.sh --skip-wms --skip-wmts

# Custom URLs
./run_all_compliance.sh --wms-url http://myserver/wms --edr-url http://myserver/edr

# Open reports in browser when done
./run_all_compliance.sh --open
```

### WMS 1.3.0 Compliance

```bash
# Basic usage (tests all profiles by default)
./run_wms_compliance.sh

# Custom WMS URL
./run_wms_compliance.sh --url http://myserver/wms

# Disable optional test profiles
./run_wms_compliance.sh --no-queryable --no-time --no-recommended

# Show all options
./run_wms_compliance.sh --help
```

**Test Profiles (enabled by default):**
- **Basic**: Core GetCapabilities, GetMap tests
- **Queryable**: GetFeatureInfo tests
- **TIME**: TIME dimension tests
- **Recommended**: OGC recommended practices

### WMTS 1.0.0 Compliance

```bash
# Basic usage
./run_wmts_compliance.sh

# Custom WMTS URL
./run_wmts_compliance.sh --url http://myserver/wmts

# Open report when done
./run_wmts_compliance.sh --open
```

### EDR 1.0 Compliance

```bash
# Basic usage
./run_edr_compliance.sh

# Custom EDR URL
./run_edr_compliance.sh --url http://myserver/edr

# Test more/all collections
./run_edr_compliance.sh --collections 5
./run_edr_compliance.sh --all-collections

# Show all options
./run_edr_compliance.sh --help
```

## Output

Results are saved to service-specific directories:

```
validation/ogc-compliance/
├── results/
│   ├── wms/
│   │   ├── report.html           # Human-readable summary
│   │   └── test-results.xml      # TEAM Engine XML results
│   ├── wmts/
│   │   ├── report.html
│   │   └── test-results.xml
│   └── edr/
│       ├── report.html
│       ├── test-run-props.xml
│       └── testng/*/testng-results.xml
├── lib/
│   └── ets-ogcapi-edr10-1.3-aio.jar  # Auto-downloaded for EDR
├── docker-compose.yml            # Docker services configuration
└── nginx-proxy.conf              # Proxy configuration for Docker
```

## Example Output

```
======================================================
           OGC COMPLIANCE TEST SUMMARY
======================================================

Service Results:

  WMS      FAILED       47 passed, 109 failed
  WMTS     FAILED       10 passed,  27 failed
  EDR      FAILED       34 passed,  18 failed

WMS Failed Tests:
  - GetCapabilities
  - GetMap-ImageFormat
  - ... and more

Overall: FAILED

Reports:
  WMS:  file:///path/to/results/wms/report.html
  WMTS: file:///path/to/results/wmts/report.html
  EDR:  file:///path/to/results/edr/report.html

Duration: 180s
```

## Architecture

### WMS/WMTS Testing (Docker-based)

WMS and WMTS tests run inside Docker using official OGC TEAM Engine containers. A key challenge is that the WMS/WMTS Capabilities documents return `localhost:8080` URLs, which Docker containers cannot reach. This is solved with an nginx proxy:

```
Docker Network (172.28.0.0/16):
+------------------------------------------------------------+
|                                                            |
|  +------------------+         +------------------+         |
|  |  TEAM Engine     |  ---->  |  nginx proxy     |  ----> Host WMS/WMTS
|  |  (WMS or WMTS)   |         |  172.28.0.10     |        (localhost:8080)
|  |  port 9093/9094  |         |  port 8080       |         |
|  +------------------+         +------------------+         |
|         |                           |                      |
|         |  URLs rewritten:          |                      |
|         |  localhost:8080 ->        |                      |
|         |  172.28.0.10:8080         |                      |
+------------------------------------------------------------+
```

The nginx proxy:
1. Forwards requests from Docker to the host's WMS/WMTS service
2. Rewrites `localhost:8080` URLs in responses to `172.28.0.10:8080`

### EDR Testing (Standalone JAR)

EDR tests use a standalone TestNG-based JAR that runs directly via Java without Docker.

## How It Works

### WMS/WMTS (Docker + TEAM Engine)
1. **Checks Docker** availability and service accessibility
2. **Starts containers** via Docker Compose (nginx proxy + TEAM Engine)
3. **Waits for TEAM Engine** to be ready (REST API health check)
4. **Runs tests** via TEAM Engine REST API
5. **Parses XML results** from the test run
6. **Generates HTML report** with pass/fail summary
7. **Stops containers** on exit (cleanup)

### EDR (Standalone)
1. **Downloads ETS JAR** from Maven Central on first run (cached in `lib/`)
2. **Checks Java 17+** installation
3. **Verifies service accessibility** before running tests
4. **Runs OGC ETS** via Java
5. **Parses TestNG results** from XML output
6. **Generates HTML report** with detailed results

## Known Limitations

### WMS/WMTS (Docker-based)

- Requires Docker to be installed and running
- Tests run against `localhost:8080` via the nginx proxy workaround
- Some tests may still fail due to network/timing issues in containerized environment

### EDR (Standalone)

- Requires Java 17+ installed
- The test suite runs fully standalone without Docker

### General

For official OGC certification, use the OGC CITE testing facility at https://cite.ogc.org/

## CI/CD Integration

```yaml
# GitHub Actions example
- name: Run OGC Compliance Tests
  run: |
    cd validation/ogc-compliance
    ./run_all_compliance.sh

- name: Upload Compliance Reports
  uses: actions/upload-artifact@v4
  if: always()
  with:
    name: ogc-compliance-reports
    path: validation/ogc-compliance/results/
```

## Troubleshooting

### Docker not running (WMS/WMTS)

```bash
# Check Docker status
docker info

# Start Docker (Linux with systemd)
sudo systemctl start docker

# Make sure you have Docker Compose
docker compose version
```

### Java not found (EDR only)

```bash
# Install Java 17+
# Ubuntu/Debian
sudo apt install openjdk-17-jre

# macOS
brew install openjdk@17

# Arch Linux
sudo pacman -S jdk17-openjdk
```

### Service not accessible

```bash
# Check if services are running
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities"
curl "http://localhost:8080/wmts?SERVICE=WMTS&REQUEST=GetCapabilities"
curl http://localhost:8083/edr

# Start services
cargo run --release -p wms-api &
cargo run --release -p edr-api &
```

### Docker image pull failures (WMS/WMTS)

```bash
# Manually pull the TEAM Engine images
docker pull ogccite/ets-wms13:latest
docker pull ogccite/ets-wmts10:latest
docker pull nginx:alpine
```

### EDR JAR download failures

```bash
# Manual download for EDR tests
mkdir -p lib
curl -L -o lib/ets-ogcapi-edr10-1.3-aio.jar \
  https://repo1.maven.org/maven2/org/opengis/cite/ets-ogcapi-edr10/1.3/ets-ogcapi-edr10-1.3-aio.jar
```

### Viewing detailed failures

```bash
# EDR: Find failed tests with details
grep -A10 'status="FAIL"' results/edr/testng/*/testng-results.xml

# WMS/WMTS: Check TEAM Engine XML results
grep 'result="6"' results/wms/test-results.xml
grep 'result="6"' results/wmts/test-results.xml

# Or view the HTML reports
xdg-open results/wms/report.html
xdg-open results/edr/report.html
```

### Container cleanup

```bash
# If containers weren't cleaned up properly
docker compose -f docker-compose.yml -p ogc-wms-test down
docker compose -f docker-compose.yml -p ogc-wmts-test down
```

## References

- [OGC WMS 1.3.0 Specification](https://www.ogc.org/standard/wms/)
- [OGC WMTS 1.0.0 Specification](https://www.ogc.org/standard/wmts/)
- [OGC API - EDR Specification](https://ogcapi.ogc.org/edr/)
- [OGC CITE (Compliance & Interoperability Testing)](https://cite.ogc.org/)
- [OGC ETS GitHub Repositories](https://github.com/opengeospatial)
