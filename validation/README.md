# OGC WMS 1.3.0 Conformance Validation Suite

Automated Docker-based setup for running OGC TEAM Engine conformance tests against your WMS 1.3.0 implementation.

## Quick Start

```bash
# 1. Copy and configure environment
cp .env.example .env
# Edit .env to set your WMS_CAPABILITIES_URL

# 2. Run validation
./validate.sh

# Or pass URL directly:
./validate.sh 'http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0'
```

## Directory Structure

```
wms-validation/
├── docker-compose.yml    # Docker services configuration
├── validate.sh           # Quick test runner script
├── .env.example          # Environment template
├── .env                  # Your configuration (create from .env.example)
├── config/               # Test configurations
├── results/              # Test results (auto-generated)
│   └── run_YYYYMMDD_HHMMSS/
│       ├── testng-results.xml
│       ├── summary.html
│       └── test-output.log
└── scripts/
    └── run-tests.sh      # Main test execution script
```

## Services

### TEAM Engine Web Interface

Start the full TEAM Engine web UI for interactive testing:

```bash
docker compose up -d teamengine
```

Access at: http://localhost:8081/teamengine/

### Automated Test Runner

Run tests automatically without the web interface:

```bash
docker compose --profile test run --rm test-runner
```

## Configuration Options

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `WMS_CAPABILITIES_URL` | Full URL to your WMS GetCapabilities endpoint | Required |
| `ETS_VERSION` | Version of the ETS test suite | `1.32` |

### Connecting to Your WMS Server

**Local development server (on host machine):**
```
WMS_CAPABILITIES_URL=http://host.docker.internal:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
```

**Another Docker container in the same network:**
```yaml
# Add your WMS to docker-compose.yml:
services:
  my-wms:
    image: your-wms-image
    networks:
      - wms-test-network
```
```
WMS_CAPABILITIES_URL=http://my-wms:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
```

**Remote server:**
```
WMS_CAPABILITIES_URL=https://maps.example.com/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
```

## Test Data Requirements

For full conformance testing, your WMS must serve the **Blue Lake test dataset**. Download from:

- **Vector (Shapefiles):** https://cite.opengeospatial.org/teamengine/about/wms13/1.3.0/site/data-wms-1.3.0.zip
- **Raster (PNG worldfiles):** https://cite.opengeospatial.org/teamengine/about/wms13/1.3.0/site/png-worldfiles-wms-1.3.0.zip

### Required Layers

| Layer Name | Type | Description |
|------------|------|-------------|
| cite:BasicPolygons | polygon | Diamond and overlapping squares |
| cite:Bridges | point | Cam Bridge |
| cite:Buildings | polygon | Buildings along Main Street |
| cite:DividedRoutes | line | Route 75 lanes |
| cite:Forests | polygon | State Forest |
| cite:Lakes | polygon | Blue Lake |
| cite:MapNeatline | line | Border of Blue Lake vicinity |
| cite:NamedPlaces | polygon | Ashton and Goose Island |
| cite:Ponds | polygon | Stock Pond pools |
| cite:RoadSegments | line | Route 5, Main Street, dirt road |
| cite:Streams | line | Cam Stream and unnamed stream |

### WMS Requirements Checklist

- [ ] Support `image/png` or `image/gif` for GetMap
- [ ] Handle GetMap requests without SERVICE parameter
- [ ] Support CRS:84 with precision to 0.0001 degrees
- [ ] Generate maps from 8x5 to 1024x768 pixels
- [ ] Default cite:Lakes style fills polygon with non-white pixels

**For GetFeatureInfo conformance:**
- [ ] Support GetFeatureInfo requests
- [ ] Polygon layers must be queryable

**For ELEVATION dimension (optional):**
- [ ] Include cite:Terrain layer (raster) or cite:Lakes depth polygons (vector)

**For TIME dimension (optional):**
- [ ] Include cite:Autos layer with temporal data

## CI/CD Integration

### GitHub Actions

```yaml
name: WMS Conformance

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Start WMS Server
        run: |
          # Start your WMS server here
          docker compose up -d my-wms-server
          sleep 30  # Wait for startup
      
      - name: Run Conformance Tests
        env:
          WMS_CAPABILITIES_URL: http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0
        run: |
          docker compose --profile test run --rm test-runner
      
      - name: Upload Results
        uses: actions/upload-artifact@v4
        if: always()
        with:
          name: wms-test-results
          path: results/
```

### GitLab CI

```yaml
wms-conformance:
  image: docker:24
  services:
    - docker:dind
  variables:
    WMS_CAPABILITIES_URL: "http://docker:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0"
  script:
    - docker compose --profile test run --rm test-runner
  artifacts:
    paths:
      - results/
    when: always
```

## Understanding Results

### TestNG Results XML

The primary output is `testng-results.xml` containing:

```xml
<testng-results total="X" passed="Y" failed="Z" skipped="W">
  <suite name="ets-wms13">
    <test name="...">
      <class name="...">
        <test-method name="..." status="PASS|FAIL|SKIP">
          <!-- Details -->
        </test-method>
      </class>
    </test>
  </suite>
</testng-results>
```

### Common Failure Categories

| Test Area | Common Issues |
|-----------|---------------|
| GetCapabilities | Schema validation, missing required elements |
| GetMap | Unsupported CRS, image format issues |
| GetFeatureInfo | Layer not queryable, incorrect response format |
| Dimensions | Missing TIME/ELEVATION support |

## Troubleshooting

### "WMS server not responding"

1. Check your WMS URL is accessible from Docker:
   ```bash
   docker run --rm curlimages/curl curl -v "YOUR_WMS_URL"
   ```

2. For local servers, use `host.docker.internal` instead of `localhost`

### "Connection refused" on Mac/Windows

Ensure Docker Desktop has "host.docker.internal" support enabled (default on recent versions).

### Test failures with valid WMS

1. Verify you're serving the Blue Lake test dataset
2. Check CRS:84 support and precision
3. Review the full test output in `results/run_*/test-output.log`

## Resources

- [OGC WMS 1.3.0 Specification](http://portal.opengeospatial.org/files/?artifact_id=14416)
- [TEAM Engine Documentation](https://github.com/opengeospatial/teamengine)
- [ETS WMS 1.3 Test Suite](https://github.com/opengeospatial/ets-wms13)
- [OGC Compliance Program](https://www.ogc.org/compliance)
- [CITE Forum (Support)](http://cite.opengeospatial.org/forum)

## License

This testing setup is provided as-is. The underlying TEAM Engine and ETS test suites are maintained by OGC.
