# OGC API EDR 1.0 Compliance Testing

Run official OGC compliance tests against the EDR API using the OGC Executable Test Suite (ETS).

## Quick Start

```bash
# Ensure EDR API is running
cargo run --release -p edr-api

# Run compliance tests
./run-ets-cli.sh

# View the results
cat results/testng/*/testng-results.xml | head -20
open results/report.html
```

## Prerequisites

- **Java 17+** (OpenJDK or similar)
- EDR API running on port 8083
- Internet connection (for first-time JAR download)

## Usage

```bash
# Run tests against local EDR API (default)
./run-ets-cli.sh

# Run tests against custom URL
./run-ets-cli.sh --url http://my-server:8083/edr

# Test more collections
./run-ets-cli.sh --collections 5

# Test all collections
./run-ets-cli.sh --all-collections

# Open HTML report in browser when done
./run-ets-cli.sh --open

# Show all options
./run-ets-cli.sh --help
```

## How It Works

The script uses the official OGC ETS (Executable Test Suite) for EDR 1.0:

1. Downloads the all-in-one JAR from Maven Central (first run only)
2. Generates a test configuration file
3. Runs tests directly via Java (no Docker required)
4. Parses results and generates an HTML report

## Test Suite Coverage

The OGC API - EDR 1.0 test suite validates these conformance classes:

| Class | Description |
|-------|-------------|
| Core | Landing page, conformance, API definition |
| Collections | Collection metadata and discovery |
| JSON | JSON encoding support |
| GeoJSON | GeoJSON encoding for responses |
| EDR GeoJSON | EDR-specific GeoJSON extensions |
| Queries | Position, area, trajectory, corridor queries |

## Output

Results are saved to the `results/` directory:

| File | Description |
|------|-------------|
| `report.html` | Human-readable HTML summary |
| `testng/*/testng-results.xml` | Detailed TestNG XML results |
| `test-run-props.xml` | Test configuration used |

## Example Output

```
================================================
    OGC API EDR 1.0 Compliance Test Suite
================================================

[OK] Java 17 found
[OK] ETS JAR found: lib/ets-ogcapi-edr10-1.3-aio.jar
[OK] EDR API is accessible
[INFO] Auto-detected API definition: http://localhost:8083/edr/api

Running OGC API EDR 1.0 Compliance Tests
========================================

Test Results
============
Results file: results/testng/.../testng-results.xml

  Total:   56
  Passed:  34
  Failed:  18
  Skipped: 4

  Pass Rate: 60%

COMPLIANCE: FAILED
```

## Configuration

You can also pass a custom properties file:

```bash
# Create custom configuration
cat > my-config.xml << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE properties SYSTEM "http://java.sun.com/dtd/properties.dtd">
<properties version="1.0">
    <entry key="iut">https://api.example.com/edr</entry>
    <entry key="apiDefinition">https://api.example.com/edr/api</entry>
    <entry key="noofcollections">-1</entry>
</properties>
EOF

# Run with custom config
java -jar lib/ets-ogcapi-edr10-1.3-aio.jar -o results my-config.xml
```

## CI/CD Integration

```yaml
# GitHub Actions example
- name: Run OGC Compliance Tests
  run: |
    cd validation/ogc-compliance
    ./run-ets-cli.sh --collections 3

- name: Upload Compliance Report
  uses: actions/upload-artifact@v4
  with:
    name: ogc-compliance-report
    path: validation/ogc-compliance/results/
```

## Troubleshooting

### Java not found

```bash
# Install Java 17+
# Ubuntu/Debian
sudo apt install openjdk-17-jre

# macOS
brew install openjdk@17

# Arch Linux
sudo pacman -S jdk17-openjdk
```

### EDR API not accessible

```bash
# Check if API is running
curl http://localhost:8083/edr

# Start the API
cargo run --release -p edr-api
```

### Test failures

Check the detailed TestNG results for failure messages:

```bash
grep -A5 'status="FAIL"' results/testng/*/testng-results.xml
```

## File Structure

```
validation/ogc-compliance/
├── run-ets-cli.sh              # Main test runner script
├── test-run-props.xml.template # Configuration template
├── lib/
│   └── ets-ogcapi-edr10-*.jar  # OGC ETS all-in-one JAR
├── results/                    # Test output (gitignored)
│   ├── report.html
│   └── testng/*/testng-results.xml
└── README.md
```

## References

- [OGC API - Environmental Data Retrieval](https://ogcapi.ogc.org/edr/)
- [OGC ETS for EDR 1.0](https://github.com/opengeospatial/ets-ogcapi-edr10)
- [OGC CITE](https://cite.ogc.org/)
- [OGC Compliance Program](https://www.ogc.org/compliance)
