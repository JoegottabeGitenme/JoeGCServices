# Bug Report: EDRGEOJSON RequirementClass uses `req/` instead of `conf/`

## Describe the bug

The `RequirementClass.EDRGEOJSON` enum in `RequirementClass.java` checks for `req/edr-geojson` instead of `conf/edr-geojson`. This causes the `validateResponseForEDRGeoJSON` test to be skipped for implementations that correctly declare the conformance class.

## To Reproduce

1. Implement an EDR API that declares `http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/edr-geojson` in the `/conformance` endpoint
2. Run the OGC ETS test suite
3. The `validateResponseForEDRGeoJSON` test is skipped with message: `Requirements class http://www.opengis.net/spec/ogcapi-edr-1/1.0/req/edr-geojson not implemented.`

### Our Test Environment

We discovered this bug while running compliance tests using the official ETS all-in-one JAR, following the documented ["Command shell (console)" method](https://github.com/opengeospatial/ets-ogcapi-edr10/blob/master/src/site/asciidoc/how-to-run-the-tests.adoc):

**ETS Version:** 1.3 (from Maven Central)

**JAR Source:**
```
https://repo1.maven.org/maven2/org/opengis/cite/ets-ogcapi-edr10/1.3/ets-ogcapi-edr10-1.3-aio.jar
```

**Test execution command:**
```bash
java -jar ets-ogcapi-edr10-1.3-aio.jar -o results/edr test-run-props.xml
```

**Test properties file (`test-run-props.xml`):**
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE properties SYSTEM "http://java.sun.com/dtd/properties.dtd">
<properties version="1.0">
    <comment>OGC API EDR 1.0 Test Run Configuration</comment>
    <entry key="iut">http://localhost:8083/edr</entry>
    <entry key="apiDefinition">http://localhost:8083/edr/api</entry>
    <entry key="noofcollections">3</entry>
</properties>
```

**Our conformance declaration includes:**
```json
{
  "conformsTo": [
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/edr-geojson",
    ...
  ]
}
```

Despite correctly declaring `conf/edr-geojson`, the test is skipped because the ETS looks for the incorrect `req/edr-geojson` URI.

## Expected behavior

The test should recognize `conf/edr-geojson` as the conformance class identifier (not `req/edr-geojson`) and execute the test.

## Bug Location

The bug is in [`src/main/java/org/opengis/cite/ogcapiedr10/conformance/RequirementClass.java` line 14](https://github.com/opengeospatial/ets-ogcapi-edr10/blob/master/src/main/java/org/opengis/cite/ogcapiedr10/conformance/RequirementClass.java#L14):

```java
GEOJSON("http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/geojson"),
EDRGEOJSON("http://www.opengis.net/spec/ogcapi-edr-1/1.0/req/edr-geojson");  // Bug: should be conf/
```

Note that `GEOJSON` correctly uses `conf/` while `EDRGEOJSON` incorrectly uses `req/`.

## Evidence from OGC EDR Specifications

### Conformance Class vs Requirements Class URI Pattern

The OGC EDR specifications clearly distinguish between conformance classes and requirements classes using different URI patterns:

| Type | URI Pattern | Purpose |
|------|-------------|---------|
| **Conformance Class** | `.../conf/...` | Identifier declared by implementations |
| **Requirements Class** | `.../req/...` | Subject of the conformance class (what is being tested) |

### OGC API - EDR 1.0 (19-086r5)

From **Table B.36 - Conformance Class "EDR GeoJSON"**:

| Field | Value |
|-------|-------|
| **Conformance Class** | `http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/edr-geojson` |
| **Target type** | Web API |
| **Requirements** | `http://www.opengis.net/spec/ogcapi-edr-1/1.0/req/edr-geojson` |
| **Class Dependency** | `http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/core` |

### OGC API - EDR 1.1 (19-086r6)

From **Conformance Class B.5: EDR GeoJSON**:

| Field | Value |
|-------|-------|
| **IDENTIFIER** | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/edr-geojson` |
| **SUBJECT** | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/req/edr-geojson` |
| **PREREQUISITE** | `http://www.opengis.net/spec/ogcapi-edr-1/1.1/conf/core` |
| **TARGET TYPE** | Web API |

### Implementations Declare Conformance Classes (`conf/`), Not Requirements Classes (`req/`)

The EDR specification explicitly states that APIs declare **conformance classes** in the `/conformance` endpoint:

> "To support 'generic' clients that want to access implementations of multiple OGC API Standards and extensions — and not 'just' a specific API server, the EDR API has to declare the conformance classes it claims to have implemented."
>
> — OGC API - EDR 1.0 (19-086r5), Section 7.2.3

### Example Conformance Response from the Specification

The specification provides this example conformance response (19-086r5, Figure 4):

```json
{
  "conformsTo": [
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-common-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-common-2/1.0/conf/collections",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/oas30",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/html",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/geojson"
  ]
}
```

Note that **all entries use `conf/`**, not `req/`. The `conformsTo` array contains conformance class identifiers, not requirements class identifiers.

### Comparison with Other Enum Values in the Same File

The inconsistency is evident when comparing `EDRGEOJSON` with other enum values in the same file:

| Enum | URI | Correct? |
|------|-----|----------|
| `CORE` | `.../conf/core` | Yes |
| `COLLECTIONS` | `.../conf/collections` | Yes |
| `JSON` | `.../conf/json` | Yes |
| `GEOJSON` | `.../conf/geojson` | Yes |
| `EDRGEOJSON` | `.../req/edr-geojson` | **No - should be `conf/`** |
| `COVERAGEJSON` | `.../conf/coveragejson` | Yes |
| `HTML` | `.../conf/html` | Yes |
| `OAS30` | `.../conf/oas30` | Yes |
| `QUERIES` | `.../conf/queries` | Yes |

## Suggested Fix

```diff
- EDRGEOJSON("http://www.opengis.net/spec/ogcapi-edr-1/1.0/req/edr-geojson");
+ EDRGEOJSON("http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/edr-geojson");
```

## Why This Bug Has Gone Undetected

### pygeoapi is the Only OGC Certified Compliant EDR Implementation

According to the [OGC Compliance Database](https://portal.ogc.org/public_ogc/compliance/compliant.php?display_opt=1&specid=1247), **pygeoapi is the only certified compliant implementation** of OGC API - Environmental Data Retrieval Standard v1.0.1:

| Organization | Product | Status | Certified |
|--------------|---------|--------|-----------|
| Open Source Geospatial Foundation | pygeoapi 0.19.0 | Official OGC Reference Implementation | 2025-01-10 |

This means pygeoapi is the sole reference implementation used for validating the ETS test suite itself.

### pygeoapi Does Not Declare `edr-geojson` Conformance

Despite being the official reference implementation, pygeoapi's CITE testing instance ([https://demo.pygeoapi.io/cite](https://demo.pygeoapi.io/cite)) does **not** declare the `edr-geojson` conformance class.

Their `/conformance` endpoint returns (as of January 2026):

```json
{
  "conformsTo": [
    "http://www.opengis.net/spec/ogcapi-common-1/1.0/conf/core",
    "http://www.opengis.net/spec/ogcapi-common-1/1.0/conf/html",
    "http://www.opengis.net/spec/ogcapi-common-1/1.0/conf/json",
    "http://www.opengis.net/spec/ogcapi-common-1/1.0/conf/landing-page",
    "http://www.opengis.net/spec/ogcapi-common-1/1.0/conf/oas30",
    "http://www.opengis.net/spec/ogcapi-common-2/1.0/conf/collections",
    "http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/core",
    ...
  ]
}
```

Note the absence of `http://www.opengis.net/spec/ogcapi-edr-1/1.0/conf/edr-geojson`.

This means pygeoapi's CITE testing **never exercises the buggy `EDRGEOJSON` code path**, allowing the bug to remain undetected.

### No Existing Issue in the ETS Repository

As of January 2026, there is no open or closed issue in the [ets-ogcapi-edr10 repository](https://github.com/opengeospatial/ets-ogcapi-edr10/issues) reporting this bug.

## Impact

This bug causes the EDR GeoJSON conformance tests to be skipped for all compliant implementations, meaning:

1. Implementations cannot validate their EDR GeoJSON support using the official test suite
2. The test suite incorrectly reports that implementations don't support EDR GeoJSON when they do
3. Potential EDR GeoJSON compliance issues in implementations go undetected
4. The bug has gone unnoticed because pygeoapi—the **only** OGC certified compliant EDR implementation and official reference implementation—does not declare this conformance class

## References

- [OGC API - EDR 1.0 (19-086r5)](https://docs.ogc.org/is/19-086r5/19-086r5.html) - Table B.36
- [OGC API - EDR 1.1 (19-086r6)](https://docs.ogc.org/is/19-086r6/19-086r6.html) - Section B.5
- [ETS Source Code](https://github.com/opengeospatial/ets-ogcapi-edr10/blob/master/src/main/java/org/opengis/cite/ogcapiedr10/conformance/RequirementClass.java#L14)
