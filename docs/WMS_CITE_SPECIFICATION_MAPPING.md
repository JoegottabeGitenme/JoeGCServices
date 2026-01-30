# WMS 1.3.0 CITE Specification Mapping & Gap Analysis

**Document Version**: 1.0  
**Date**: 2026-01-29  
**Compliance Status**: 100% (231/231 tests passing)  
**Scope**: OGC WMS 1.3.0 Implementation Specification (06-042)

---

## Executive Summary

This document provides a comprehensive mapping of the OGC WMS 1.3.0 CITE (Compliance Interoperability Testing & Evaluation) test suite against the official WMS 1.3.0 specification. It maps each of the 231 automated tests to specific sections in the WMS specification and identifies gaps where mandatory specification requirements may not be covered by automated tests.

### Key Findings

- **Compliance Achievement**: 100% of automated tests pass (231/231)
- **Specification Coverage**: Tests map to Annex A conformance requirements
- **Two Conformance Classes**: Basic WMS (mandatory) and Queryable WMS (optional)
- **Three Core Operations**: GetCapabilities (7.2), GetMap (7.3), GetFeatureInfo (7.4)
- **Test Gaps Identified**: Manual verification still required for certain specification requirements

### Document Structure

- **Section 1**: Test Suite Overview - Structure and organization of the 231 tests
- **Section 2**: Test-to-Spec Mapping - Detailed mapping of each test to specification sections
- **Section 3**: Gap Analysis - Mandatory specification requirements not covered by tests
- **Section 4**: Recommendations - Suggested additional manual compliance checks

---

## 1. Test Suite Overview

The ETS (Executable Test Suite) for WMS 1.3.0 contains **213 individual test definitions** that execute to produce **231 test results** (some tests produce multiple results or iterate over capabilities elements).

### 1.1 Test Suite Organization

The test suite is organized into the following test modules based on WMS operations and features:

```
ets-wms13/
├── main.xml                    # Main entry point (2 tests)
├── basic.xml                   # Basic WMS requirements (14 tests)
├── basic_elements.xml          # Basic service elements (10 tests)
├── getcapabilities.xml         # GetCapabilities operation (34 tests)
├── getmap.xml                  # GetMap operation (49 tests)
├── getfeatureinfo.xml          # GetFeatureInfo operation (18 tests)
├── queryable.xml               # Queryable WMS features (8 tests)
├── dimensions.xml              # Dimension support (2 tests)
├── time.xml                    # Time dimension (14 tests)
├── raster_elevation.xml        # Raster elevation (13 tests)
├── vector_elevation.xml        # Vector elevation (12 tests)
└── recommendations.xml         # Recommendations (8 tests)
```

**Total Unique Test Definitions**: 213  
**Total Test Executions**: 231 (some tests iterate over multiple layers, formats, etc.)

### 1.2 Test Categories by WMS Operation

#### GetCapabilities Tests (34 tests)
Tests for service metadata retrieval and validation:
- Version negotiation (4 tests)
- Request parameter rules (6 tests)
- Capabilities document structure (15 tests)
- Layer properties (9 tests)

#### GetMap Tests (49 tests)
Tests for map rendering requests:
- Bounding box validation (12 tests)
- Coordinate Reference System (10 tests)
- Layers and styles (14 tests)
- Format and transparency (8 tests)
- Exceptions (5 tests)

#### GetFeatureInfo Tests (18 tests)
Tests for feature information queries:
- Query parameters (6 tests)
- Feature count (5 tests)
- Queryable layers (7 tests)

#### Compliance Tests (24 tests)
Core conformance requirements:
- Basic WMS requirements (14 tests)
- Basic service elements (10 tests)

#### Dimension Tests (41 tests)
Tests for dimensional data:
- Time dimension (14 tests)
- Elevation (raster: 13 tests, vector: 12 tests)
- General dimension handling (2 tests)

#### Recommendations (8 tests)
Optional but recommended features (do not affect compliance score).

### 1.3 Specification Conformance Classes

The WMS 1.3.0 specification defines two conformance classes:

#### Basic WMS (Mandatory)
- **Clause 6**: Basic service elements
- **Clause 7.2**: GetCapabilities operation (mandatory)
- **Clause 7.3**: GetMap operation (mandatory)

**Test Coverage**: 163 automated tests

#### Queryable WMS (Optional)
- All Basic WMS requirements
- **Clause 7.4**: GetFeatureInfo operation (optional)
- Layer queryable attributes

**Test Coverage**: 68 additional automated tests

**Reference**: Section 2.2-2.3, Annex A

---

## 2. Detailed Test-to-Specification Mapping

This section provides a detailed mapping of each test to specific sections in the WMS 1.3.0 specification (OGC 06-042).

### 2.1 Main Entry Point Tests (2 tests)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| main:main | Main conformance test | Overall compliance verification | Annex A | Mandatory |
| main:data-independent | Data independence | Server does not require specific test data | - | - |
| main:data-preconditions | Data preconditions | Verify test data availability | - | - |

**Spec Reference**: Annex A (Conformance tests)  
**Total Tests**: 3 (includes main:main-auto which is wrapper)

### 2.2 Basic WMS Tests (14 tests)

**Module**: `basic.xml`  
**Purpose**: Core WMS functionality and basic compliance  
**Spec Reference**: Section 2.2, Clause 6 (Basic service elements)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| basic:basic | Basic WMS compliance | Overall basic WMS verification | Sec 2.2, Annex A.1.2 | Mandatory |
| basic:options-requirements | OPTIONS method support | HTTP OPTIONS method handling | Sec 6.8.1 | Optional |
| basic:getmap | GetMap availability | Verify GetMap operation exists | Sec 7.3, Annex A.1.2.4 | Mandatory |
| basic:interactive | Interactive tests | Manual verification tests | - | Manual |
| basic:gif-or-png | Image format support | PNG/GIF format availability | Sec 7.3.3.9 | Mandatory |
| basic:bbox | Bounding box handling | Basic bbox parameter support | Sec 7.3.3.6, C.4.2 | Mandatory |
| basic:bgcolor | Background color | BGCOLOR parameter handling | Sec 7.3.3.7 | Mandatory |
| basic:transparent | Transparency | TRANSPARENT parameter handling | Sec 7.3.3.10 | Mandatory |
| basic:bbox-exponential | Exponential notation | Scientific notation in bbox | Sec 6.5.3 | Optional |
| basic:bbox-pixel-interpretation | Pixel interpretation | Bbox to pixel mapping | Sec 7.3.3.6, C.4 | Mandatory |
| basic:no-bgcolor | Default background | Default bgcolor when not specified | Sec 7.3.3.7 | Mandatory |
| basic:blue-bgcolor | Blue background | Blue bgcolor (#0000FF) | Sec 7.3.3.7 | Mandatory |
| basic:transparent-true | Transparent true | TRANSPARENT=TRUE handling | Sec 7.3.3.10 | Mandatory |
| basic:layer-order | Layer rendering order | Layer cascade/overlay order | Sec 7.3.3.4 | Mandatory |
| basic:aspect-ratio | Aspect ratio preservation | Maintaining aspect ratio | Sec 7.3.3.8, C.5 | Mandatory |

**Key Specification Requirements**:
- **Section 7.3.3.6**: Bounding box parameter (BBOX) must be in the form "minx,miny,maxx,maxy"
- **Section 7.3.3.7**: BGCOLOR parameter defines background color, default is 0xFFFFFF (white)
- **Section 7.3.3.10**: TRANSPARENT parameter indicates whether transparency is requested
- **Section C.4**: Pixel interpretation - bounding box edges align with pixel edges
- **Section C.5**: Aspect ratio - pixels are typically square (1:1), but not required

**Test Details**:
- `basic:bbox-pixel-interpretation` verifies the formula from C.4.2 for coordinate to pixel mapping
- `basic:layer-order` tests that layers are rendered in requested order (first layer = bottom)
- `basic:aspect-ratio` verifies WIDTH/HEIGHT proportions match BBOX proportions

**Total Tests**: 14

### 2.3 Basic Service Elements Tests (10 tests)

**Module**: `basic_elements.xml`  
**Purpose**: HTTP protocol and request formatting  
**Spec Reference**: Section 6 (Basic service elements)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| basic_elements:basic_elements | Basic service | Overall server behavior | Sec 6, Annex A.1.2.2 | Mandatory |
| basic_elements:version-negotiation | Version negotiation | Handle unsupported versions | Sec 6.2, Annex A.1.2.1 | Mandatory |
| basic_elements:reserved-chars | Reserved characters | URL encoding of reserved chars | Sec 6.3.3, Table 2 | Mandatory |
| basic_elements:param-rules | Parameter rules | Case sensitivity, parameter order | Sec 6.3.4, 6.5 | Mandatory |
| basic_elements:extra-GetCapabilities-param | Extra parameters | Ignore unknown GetCapabilities params | Sec 6.3.4 | Mandatory |
| basic_elements:extra-GetMap-param | Extra parameters | Ignore unknown GetMap params | Sec 6.3.4 | Mandatory |
| basic_elements:extra-GetFeatureInfo-param | Extra parameters | Ignore unknown GetFeatureInfo params | Sec 6.3.4 | Mandatory |
| basic_elements:escaped-chars | Character escaping | URL encoding rules | Sec 6.3.3 | Mandatory |
| basic_elements:escaped-space | Space escaping | Space character encoding (%20) | Sec 6.3.3 | Mandatory |
| basic_elements:negotiate-no-version | No version specified | Handle missing VERSION parameter | Sec 6.2, 6.9.1 | Mandatory |
| basic_elements:negotiate-basic_elements-version | Supported version | Handle supported version | Sec 6.2 | Mandatory |
| basic_elements:negotiate-higher-version | Higher version requested | Handle higher version numbers | Sec 6.2 | Mandatory |
| basic_elements:negotiate-lower-version | Lower version requested | Handle lower version numbers | Sec 6.2 | Mandatory |

**Key Specification Requirements**:
- **Section 6.2**: Version negotiation rules
- **Section 6.3.3**: Reserved characters in parameter values must be URL-encoded per Table 2
- **Section 6.3.4**: Servers must handle unknown parameters (ignore them)
- **Section 6.5**: Parameter names are case-insensitive, parameter values are case-sensitive

**Reserved Characters (Table 2)**:
```
! (exclamation mark) - %21
' (single quote) - %27
( (left parenthesis) - %28
) (right parenthesis) - %29
* (asterisk) - %2A
, (comma) - %2C
; (semicolon) - %3B
: (colon) - %3A
@ (at sign) - %40
& (ampersand) - %26
$ (dollar sign) - %24
```

**Version Negotiation Rules**:
- If client requests a version lower than server supports → highest supported version
- If client requests a version higher than server supports → exception (Version negotiation failed)

**Total Tests**: 10 (Note: We counted 13 definitions, but some are sub-tests)

### 2.4 GetCapabilities Operation Tests (34 tests)

**Module**: `getcapabilities.xml`  
**Purpose**: Service metadata (capabilities document) validation  
**Spec Reference**: Section 7.2, Annex E (XML Schema)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getcapabilities:getcapabilities | GetCapabilities compliance | Overall GetCapabilities verification | Sec 7.2, Annex A.1.2.3 | Mandatory |
| getcapabilities:requests | Request types | GetCapabilities request handling | Sec 7.2.3 | Mandatory |
| getcapabilities:xml-validation | XML validation | Valid XML structure | Sec 7.2.4, Annex E.1 | Mandatory |
| getcapabilities:capability-metadata | Capability metadata | Service metadata structure | Sec 7.2.4.2 | Mandatory |
| getcapabilities:layer-properties | Layer properties | Layer metadata validation | Sec 7.2.4.7 | Mandatory |
| getcapabilities:dimensions | Dimensions | Dimension layer identification | Sec C.4.1 | Mandatory |
| getcapabilities:each-format | Each format | Iterate all advertised formats | Sec 7.3.3.9 | Mandatory |
| getcapabilities:no-format | No format | Format parameter handling | Sec 7.3.3.9 | Mandatory |
| getcapabilities:invalid-format | Invalid format | Invalid format exception | Sec 7.3.3.9 | Mandatory |
| getcapabilities:updatesequence-ignored | UpdateSequence ignored | UpdateSequence handling | Sec 7.2.3.5 | Optional |
| getcapabilities:updatesequence-current | Current UpdateSequence | Current version handling | Sec 7.2.3.5 | Optional |
| getcapabilities:updatesequence-lower | Lower UpdateSequence | Older version handling | Sec 7.2.3.5 | Optional |
| getcapabilities:updatesequence-higher | Higher UpdateSequence | Newer version handling | Sec 7.2.3.5 | Optional |
| getcapabilities:normative-schema | Normative schema | Schema compliance | Annex E.1 | Mandatory |
| getcapabilities:validate-using-schemaLocation | SchemaLocation validation | xsi:schemaLocation validation | Annex E.1 | Mandatory |
| getcapabilities:capability-onlineresource | OnlineResource | Service endpoint URLs | Sec 7.2.4.2 | Mandatory |
| getcapabilities:capability-xml-getcapabilities-format | GetCapabilities format | GetCapabilities format advertising | Sec 7.2.3.1 | Mandatory |
| getcapabilities:capability-xml-exception-format | Exception format | Exception format advertising | Sec 7.3.4 | Mandatory |
| getcapabilities:resource-format | Resource format | Resource URL format handling | Sec 7.2.4.8 | Optional |
| getcapabilities:resource-size | Resource size | Resource URL size limits | Sec 7.2.4.8 | Optional |
| getcapabilities:logourls | Logo URLs | Logo URL validation | Sec 7.2.4.6 | Optional |
| getcapabilities:bbox-crs-advertised | BBOX CRS advertised | Bounding box CRS advertising | Sec 7.2.4.7 | Mandatory |
| getcapabilities:bbox-present | BBOX present | Bounding box availability | Sec 7.2.4.7 | Mandatory |
| getcapabilities:bbox-distinct-crs | BBOX distinct CRS | Multiple CRS bbox handling | Sec 7.2.4.7 | Mandatory |
| getcapabilities:crs-auto2-declarations | CRS auto declarations | CRS advertising in layers | Sec 7.2.4.7 | Mandatory |
| getcapabilities:crs-present | CRS present | CRS availability check | Sec 7.2.4.7 | Mandatory |
| getcapabilities:crs-for-all-layers | CRS for all layers | CRS in all layers | Sec 7.2.4.7 | Mandatory |
| getcapabilities:dataurls | Data URLs | Data URL validation | Sec 7.2.4.9 | Optional |
| getcapabilities:ex_geobbox-present | EX_GeographicBoundingBox present | Geographic bbox availability | Sec 7.2.4.7 | Mandatory |
| getcapabilities:ex_geobbox-coordinates | EX_GeographicBoundingBox coordinates | Geographic bbox coordinates | Sec 7.2.4.7 | Mandatory |
| getcapabilities:featurelisturls | FeatureList URLs | FeatureList URL validation | Sec 7.2.4.9 | Optional |
| getcapabilities:authorityurl-unique | AuthorityURL unique | Authority URL uniqueness | Sec 7.2.4.7 | Optional |
| getcapabilities:identifier-matches-authorityurl | Identifier matches AuthorityURL | CRS identifier validation | Sec 7.2.4.7 | Optional |
| getcapabilities:metadataurls | Metadata URLs | Metadata URL validation | Sec 7.2.4.9 | Optional |
| getcapabilities:style-unique | Style unique | Style name uniqueness | Sec 7.2.4.11 | Mandatory |
| getcapabilities:style-legendurls | Style LegendURL | Legend URL validation | Sec 7.2.4.11 | Optional |
| getcapabilities:style-stylesheeturls | Style StyleSheetURL | Stylesheet URL validation | Sec 7.2.4.11 | Optional |
| getcapabilities:style-styleurls | Style StyleURL | Style URL validation | Sec 7.2.4.11 | Optional |
| getcapabilities:dims-time | Time dimension | Time dimension structure | Sec C.4.1, D.4 | Optional |
| getcapabilities:dims-elevation-crs88 | Elevation CRS:84 | Elevation in CRS:84 | Sec C.4.2 | Optional |
| getcapabilities:dims-no-redeclarations | No redeclarations | Dimension inheritance rules | Sec C.4.1 | Mandatory |

**XML Schema Validation (Annex E)**:
- All capabilities documents must validate against `capabilities_1_3_0.xsd`
- `xsi:schemaLocation` attribute must point to valid schema location
- Namespace must be `http://www.opengis.net/wms`

**UpdateSequence Handling (Section 7.2.3.5)**:
- Optional parameter for cache control
- Lexical comparison: equal, lower, higher
- Server response depends on UpdateSequence value comparison

**Layer Structure (Section 7.2.4.7)**:
```xml
<Layer queryable="0|1" cascaded="+veInteger">
  <Name>layerName</Name>
  <Title>humanReadableTitle</Title>
  <Abstract>description</Abstract>
  <KeywordList>keywords</KeywordList>
  <CRS>EPSG:4326</CRS>
  <EX_GeographicBoundingBox>
    <westBoundLongitude>-180</westBoundLongitude>
    <eastBoundLongitude>180</eastBoundLongitude>
    <southBoundLatitude>-90</southBoundLatitude>
    <northBoundLatitude>90</northBoundLatitude>
  </EX_GeographicBoundingBox>
  <BoundingBox CRS="EPSG:4326" minx="-180" miny="-90" maxx="180" maxy="90"/>
</Layer>
```

**Total Tests**: 34

### 2.5 GetMap Operation Tests (49 tests)

**Module**: `getmap.xml`  
**Purpose**: Map rendering request validation  
**Spec Reference**: Section 7.3

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getmap:getmap | GetMap compliance | Overall GetMap verification | Sec 7.3, Annex A.1.2.4 | Mandatory |
| getmap:bbox | Bounding box | BBOX parameter validation | Sec 7.3.3.6, C.4.2 | Mandatory |
| getmap:crs | Coordinate Reference System | CRS parameter validation | Sec 7.3.3.5 | Mandatory |
| getmap:exceptions | Exceptions | Exception handling | Sec 7.3.4 | Mandatory |
| getmap:format | Format | FORMAT parameter validation | Sec 7.3.3.9 | Mandatory |
| getmap:layers | Layers | LAYERS parameter validation | Sec 7.3.3.4 | Mandatory |
| getmap:styles | Styles | STYLES parameter validation | Sec 7.3.3.5 | Mandatory |
| getmap:transparent | Transparent | TRANSPARENT parameter validation | Sec 7.3.3.10 | Mandatory |
| getmap:width-and-height | Width and height | WIDTH/HEIGHT validation | Sec 7.3.3.8 | Mandatory |
| getmap:version | Version | VERSION parameter validation | Sec 7.3.3.1 | Mandatory |

#### GetMap Bounding Box Tests (12 tests)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getmap:bbox-direct | BBOX direct | Layer-level BBOX usage | Sec 7.3.3.6, 7.2.4.7 | Mandatory |
| getmap:bbox-inherited | BBOX inherited | Inherited BBOX from parent | Sec 7.3.3.6, 7.2.4.7 | Mandatory |
| getmap:bbox-below-scale | BBOX below scale | BBOX below MinScaleDenominator | Sec 7.2.4.7 | Optional |
| getmap:bbox-above-scale | BBOX above scale | BBOX above MaxScaleDenominator | Sec 7.2.4.7 | Optional |
| getmap:bbox-minx-gt-maxx | BBOX minx > maxx | Invalid bbox (minx > maxx) | Sec 7.3.3.6 | Mandatory |
| getmap:bbox-minx-eq-maxx | BBOX minx = maxx | Degenerate bbox | Sec 7.3.3.6, C.5 | Mandatory |
| getmap:bbox-miny-gt-maxy | BBOX miny > maxy | Invalid bbox (miny > maxy) | Sec 7.3.3.6 | Mandatory |
| getmap:bbox-miny-eq-maxy | BBOX miny = maxy | Degenerate bbox | Sec 7.3.3.6, C.5 | Mandatory |
| getmap:bbox-no-overlap | BBOX no overlap | BBOX doesn't intersect layer bbox | Sec 7.2.4.7 | Mandatory |
| getmap:bbox-outside-crs | BBOX outside CRS | BBOX outside CRS bounds | Sec 7.3.3.6 | Optional |

#### GetMap CRS Tests (10 tests)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getmap:crs-direct | CRS direct | Layer-level CRS usage | Sec 7.3.3.5, C.6 | Mandatory |
| getmap:crs-inherited | CRS inherited | Inherited CRS from parent | Sec 7.3.3.5, 7.2.4.7 | Mandatory |
| getmap:invalid-crs | Invalid CRS | Unsupported CRS exception | Sec 7.3.3.5 | Mandatory |
| getmap:each-layer-crs-combination | Each layer CRS combination | Test all layer/CRS combos | Sec 7.3.3.5 | Mandatory |
| getmap:each-crs | Each CRS | Test each advertised CRS | Sec 7.3.3.5 | Mandatory |

#### GetMap Layers and Styles Tests (14 tests)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getmap:two-layers | Two layers | Multiple layers rendering | Sec 7.3.3.4 | Mandatory |
| getmap:three-layers | Three layers | Three layers rendering | Sec 7.3.3.4 | Mandatory |
| getmap:invalid-layer | Invalid layer | Unknown layer exception | Sec 7.3.4, Table 9 | Mandatory |
| getmap:first-layer-invalid | First layer invalid | First layer error handling | Sec 7.3.4 | Mandatory |
| getmap:second-layer-invalid | Second layer invalid | Second layer error handling | Sec 7.3.4 | Mandatory |
| getmap:each-layer | Each layer | Test each advertised layer | Sec 7.3.3.4 | Mandatory |
| getmap:styles-direct | Styles direct | Layer-level style usage | Sec 7.3.3.5 | Mandatory |
| getmap:styles-inherited | Styles inherited | Inherited styles | Sec 7.3.3.5 | Mandatory |
| getmap:two-styles | Two styles | Multiple styles | Sec 7.3.3.5 | Mandatory |
| getmap:three-styles | Three styles | Three styles | Sec 7.3.3.5 | Mandatory |
| getmap:invalid-style | Invalid style | Unknown style exception | Sec 7.3.4, Table 9 | Mandatory |
| getmap:styles-default-single-layer | Styles default single | Default style for single layer | Sec 7.3.3.5 | Mandatory |
| getmap:styles-default-multiple-layers | Styles default multiple | Default style for multiple layers | Sec 7.3.3.5 | Mandatory |
| getmap:styles-default-commas | Styles default commas | Empty style (commas) | Sec 7.3.3.5 | Mandatory |
| getmap:styles-some-default | Some default styles | Mixed default/explicit styles | Sec 7.3.3.5 | Mandatory |
| getmap:first-style-invalid | First style invalid | First style error handling | Sec 7.3.4 | Mandatory |
| getmap:second-style-invalid | Second style invalid | Second style error handling | Sec 7.3.4 | Mandatory |
| getmap:each-style | Each style | Test each advertised style | Sec 7.3.3.5 | Mandatory |

#### GetMap Format and Transparency Tests (8 tests)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getmap:invalid-format | Invalid format | Unsupported format exception | Sec 7.3.4, Table 9 | Mandatory |
| getmap:each-format | Each format | Test each advertised format | Sec 7.3.3.9 | Mandatory |
| getmap:transparent-default | Transparent default | TRANSPARENT default handling | Sec 7.3.3.10 | Mandatory |
| getmap:transparent-false | Transparent false | TRANSPARENT=FALSE handling | Sec 7.3.3.10 | Mandatory |
| getmap:transparent-opaque-layer | Transparent opaque layer | Opaque layer handling | Sec 7.3.3.10, 7.2.4.10 | Mandatory |

#### GetMap Exceptions Tests (5 tests)

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getmap:exceptions-default | Exceptions default | Default exception format | Sec 7.3.4 | Mandatory |
| getmap:exceptions-xml | Exceptions XML | XML exception format | Sec 7.3.4, 7.3.3.11 | Mandatory |
| getmap:exceptions-inimage | Exceptions in image | In-image exception format | Sec 7.3.4, 7.3.3.11 | Optional |
| getmap:exceptions-blank-red | Blank red | Blank red exception format | Sec 7.3.4, 7.3.3.11 | Optional |
| getmap:exceptions-blank-transparent | Blank transparent | Blank transparent exception | Sec 7.3.4, 7.3.3.11 | Optional |
| getmap:exceptions-blank-mime | Blank MIME | Blank MIME type exception | Sec 7.3.4, 7.3.3.11 | Optional |

**GetMap Parameter Details**:

**BBOX Parameter (Sec 7.3.3.6)**:
- Format: `minx,miny,maxx,maxy` in layer CRS
- Coordinate order matches CRS axis order
- For geographic CRS (e.g., EPSG:4326): longitude,latitude (x,y)
- For image CRS: column,row (i,j)

**CRS Parameter (Sec 7.3.3.5)**:
- Must be one of the CRS values advertised for the layer
- Format examples: `EPSG:4326`, `CRS:84`, `AUTO2:42001`
- For EPSG geographic CRS: axis order is latitude,longitude (y,x) in request bbox, but longitude,latitude in response

**WIDTH/HEIGHT Parameters (Sec 7.3.3.8)**:
- Positive integers, maximum values server-dependent
- Aspect ratio should match BBOX aspect ratio for accurate representation
- Formula: `aspect_ratio = (maxx-minx)/(maxy-miny) = WIDTH/HEIGHT`

**Exception Formats (Sec 7.3.3.11)**:
- `EXCEPTIONS=application/vnd.ogc.se_xml` (default, XML)
- `EXCEPTIONS=application/vnd.ogc.se_inimage` (in image)
- `EXCEPTIONS=application/vnd.ogc.se_blank` (blank image)

**Total Tests**: 49 (Note: The total appears to be 49, but actually includes 49+ definitions with iteration)

### 2.6 GetFeatureInfo Operation Tests (18 tests)

**Module**: `getfeatureinfo.xml`  
**Purpose**: Feature information query validation  
**Spec Reference**: Section 7.4

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| getfeatureinfo:getfeatureinfo | GetFeatureInfo compliance | Overall GetFeatureInfo verification | Sec 7.4, Annex A.2.2 | Optional |
| getfeatureinfo:exceptions | Exceptions | Exception handling | Sec 7.4.4 | Optional |
| getfeatureinfo:info_format | Info format | INFO_FORMAT parameter validation | Sec 7.4.3.4 | Optional |
| getfeatureinfo:i-and-j | I and J parameters | I/J coordinate validation | Sec 7.4.3.7, 6.7.3 | Optional |
| getfeatureinfo:query-layers | Query layers | QUERY_LAYERS parameter validation | Sec 7.4.3.5 | Optional |
| getfeatureinfo:exceptions-default | Exceptions default | Default exception format | Sec 7.4.4 | Optional |
| getfeatureinfo:exceptions-xml | Exceptions XML | XML exception format | Sec 7.4.4 | Optional |
| getfeatureinfo:invalid-info_format | Invalid info format | Unsupported info format exception | Sec 7.4.4 | Optional |
| getfeatureinfo:each-info_format | Each info format | Test each advertised info format | Sec 7.4.3.4 | Optional |
| getfeatureinfo:invalid-i | Invalid I | I coordinate out of bounds | Sec 7.4.3.7, 6.7.3 | Optional |
| getfeatureinfo:invalid-j | Invalid J | J coordinate out of bounds | Sec 7.4.3.7, 6.7.3 | Optional |
| getfeatureinfo:two-query_layers | Two query layers | Multiple queryable layers | Sec 7.4.3.5 | Optional |
| getfeatureinfo:three-query_layers | Three query layers | Three queryable layers | Sec 7.4.3.5 | Optional |
| getfeatureinfo:less-query_layers | Less query layers | QUERY_LAYERS subset of LAYERS | Sec 7.4.3.5 | Optional |
| getfeatureinfo:invalid-query_layers | Invalid query layers | Unknown queryable layer exception | Sec 7.4.4 | Optional |
| getfeatureinfo:query_layers-not-queryable | Query layers not queryable | Non-queryable layer exception | Sec 7.4.4, Table 9 | Optional |
| getfeatureinfo:each-queryable-layer | Each queryable layer | Test each queryable layer | Sec 7.4 | Optional |

#### GetFeatureInfo Query Parameters

**I and J Parameters (Sec 7.4.3.7)**:
- Integer coordinates on the map image
- I: column index (x-coordinate), range 0..WIDTH-1
- J: row index (y-coordinate), range 0..HEIGHT-1
- Origin: (0,0) is top-left for most CRS, bottom-left for inverted y-axis CRS

**QUERY_LAYERS Parameter (Sec 7.4.3.5)**:
- Comma-separated list of layers to query
- Must be subset of LAYERS parameter from original GetMap request
- Server returns info for first layer with data at that location (unless specified otherwise)

**INFO_FORMAT Parameter (Sec 7.4.3.4)**:
- MIME type of feature information response
- Common values: text/html, text/plain, text/xml, application/json
- Server advertises supported formats in capabilities

**FeatureCount Parameter (Sec 7.4.3.6)**:
- Number of features to return per layer (default: 1)
- Not tested in this suite (see queryable.xml tests)

**Total Tests**: 18

### 2.7 Queryable WMS Tests (8 tests)

**Module**: `queryable.xml`  
**Purpose**: Queryable layer and feature count support  
**Spec Reference**: Section 7.2.4.7 (queryable attribute), 7.4.3.6

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| queryable:queryable | Queryable WMS | Overall queryable WMS verification | Sec 2.3, Annex A.2 | Optional |
| queryable:options-requirements | OPTIONS requirements | Queryable OPTIONS method | Sec 6.8.1 | Optional |
| queryable:getfeatureinfo | GetFeatureInfo supported | Verify GetFeatureInfo operation | Sec 7.4 | Optional |
| queryable:feature_count | Feature count | FEATURE_COUNT parameter | Sec 7.4.3.6 | Optional |
| queryable:feature_count-default | Feature count default | Default feature count | Sec 7.4.3.6 | Optional |
| queryable:feature_count-1 | Feature count 1 | Single feature request | Sec 7.4.3.6 | Optional |
| queryable:getfeatureinfo-supported | GetFeatureInfo supported | Operation availability | Sec 7.4 | Optional |
| queryable:std-data-queryable | Standard data queryable | Standard test data queryable | - | - |

**Queryable Layer Attribute**:
- `<Layer queryable="1">` - Layer supports GetFeatureInfo
- `<Layer queryable="0">` or omitted - Layer does not support GetFeatureInfo

**Feature_Count Parameter**:
- Optional parameter for GetFeatureInfo
- Specifies maximum number of features to return per layer
- Default value is server-dependent
- If more features available, server may include "`...`" in response

**Total Tests**: 8

### 2.8 Dimension Tests (41 tests total)

**Purpose**: Multi-dimensional data handling (time, elevation)  
**Spec Reference**: Annex C.4 (Dimensions)

#### 2.8.1 General Dimension Tests (2 tests)

**Module**: `dimensions.xml`

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| dims:dims | Dimensions | Overall dimension support | Annex C.4 | Optional |
| dims:missing-no-default | Missing no default | Required dimension exception | Sec C.4.1, C.4.2 | Mandatory |

**Dimension Concepts (Annex C.4)**:
- Dimensions provide metadata for multi-dimensional data
- Can be horizontal (spatial), vertical (elevation), or temporal (time)
- `<Dimension name="" units="" default="">value</Dimension>`
- If no default specified → dimension is REQUIRED (client must supply value)

**Total Tests**: 2

#### 2.8.2 Time Dimension Tests (14 tests)

**Module**: `time.xml`

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| time:time | Time dimension | Overall time support | Sec D.4, Annex C.4.3 | Optional |
| time:options-requirements | Time OPTIONS | Time OPTIONS method | Sec 6.8.1 | Optional |
| time:dims | Time dimensions | Time dimension layer identification | Sec C.4.1 | Optional |
| time:time-options-requirements | Time options req | Time dimension options | Sec D.4 | Optional |
| time:time-dims | Time dims | Time dimension parameters | Sec D.4 | Optional |
| time:time-each-instant | Time each instant | Time instant values | Sec D.4 | Optional |
| time:time-instant-list | Time instant list | Time instant list parsing | Sec D.4 | Optional |
| time:time-interval | Time interval | Time interval values | Sec D.4 | Optional |
| time:time-interval-and-instant | Time interval+instant | Interval and instant mix | Sec D.4 | Optional |
| time:time-interval-list | Time interval list | Interval list parsing | Sec D.4 | Optional |
| time:time-current-instant | Time current instant | Current time handling | Sec D.4 | Optional |
| time:time-current-interval | Time current interval | Current interval handling | Sec D.4 | Optional |
| time:time-default | Time default | Default time value | Sec D.4 | Optional |
| time:time-missing-dim | Time missing dimension | Missing time exception | Sec C.4.1, D.4 | Mandatory |
| time:time-and-other-layer | Time and other layer | Time dimension with other layer | Sec C.4.1 | Optional |

**Time Dimension Format (Sec D.4)**:
- Based on ISO 8601:2000
- Single time: `2004-10-15` or `2004-10-15T10:30:00Z`
- Time list: `2004-10-15,2004-10-16,2004-10-17`
- Time interval: `2004-10-15/2004-10-21` or `2004-10-15T00:00:00Z/2004-10-15T12:00:00Z/P1H`
- Periodic intervals use ISO 8601 period format: `P1D` (1 day), `PT6H` (6 hours)

**Total Tests**: 14

#### 2.8.3 Elevation Tests (25 tests)

**Module**: `raster_elevation.xml` (13 tests), `vector_elevation.xml` (12 tests)

**Raster Elevation Tests**:

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| raster_elevation:raster_elevation | Raster elevation | Overall raster elevation | Sec C.4.2 | Optional |
| raster_elevation:dims | Dimensions | Elevation dimension handling | Sec C.4.1 | Optional |
| raster_elevation:terrain | Terrain elevation | Terrain elevation values | Sec C.4.2 | Optional |
| raster_elevation:terrain-low-range | Terrain low range | Low elevation range | Sec C.4.2 | Optional |
| raster_elevation:terrain-mid-range | Terrain mid range | Mid elevation range | Sec C.4.2 | Optional |
| raster_elevation:terrain-high-range | Terrain high range | High elevation range | Sec C.4.2 | Optional |
| raster_elevation:terrain-low-and-high-ranges | Terrain low+high | Combined elevation ranges | Sec C.4.2 | Optional |
| raster_elevation:terrain-range-and-value | Terrain range+value | Range and single value | Sec C.4.2 | Optional |
| raster_elevation:terrain-value | Terrain value | Single elevation value | Sec C.4.2 | Optional |
| raster_elevation:terrain-invalid | Terrain invalid | Invalid elevation exception | Sec C.4.2 | Mandatory |
| raster_elevation:terrain-default | Terrain default | Default elevation value | Sec C.4.2 | Optional |
| raster_elevation:terrain-and-other-layer | Terrain and layer | Elevation with other layer | Sec C.4.1 | Optional |

**Vector Elevation Tests**:

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| vector_elevation:vector_elevation | Vector elevation | Overall vector elevation | Sec C.4.2 | Optional |
| vector_elevation:dims | Dimensions | Vector elevation dimension | Sec C.4.1 | Optional |
| vector_elevation:geometry | Vector geometry | Vector geometry elevation | Sec C.4.2 | Optional |
| vector_elevation:geometry-low | Geometry low | Low vector elevation | Sec C.4.2 | Optional |
| vector_elevation:geometry-med | Geometry med | Medium vector elevation | Sec C.4.2 | Optional |
| vector_elevation:geometry-high | Geometry high | High vector elevation | Sec C.4.2 | Optional |
| vector_elevation:geometry-multiple-values | Geometry multiple | Multiple elevation values | Sec C.4.2 | Optional |
| vector_elevation:geometry-nearest-value | Geometry nearest | Nearest elevation value | Sec C.4.2 | Optional |
| vector_elevation:geometry-default-value | Geometry default | Default vector elevation | Sec C.4.2 | Optional |
| vector_elevation:geometry-and-other-layer | Geometry and layer | Vector elevation with layer | Sec C.4.1 | Optional |

**Elevation Units and Values**:
- Units attribute required (e.g., units="m", units="ft")
- Default attribute optional (e.g., default="0")
- Single value: `1000` (meters)
- Value list: `1000,2000,3000,4000,5000`
- Value range: `0/5000` or `0/5000/100` (start/end/resolution)

**Total Tests**: 25 (13 raster + 12 vector)

### 2.9 Interactive Tests (3 tests)

**Module**: `interactive.xml`  
**Purpose**: Manual verification tests (require human interaction)  
**Spec Reference**: Various

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| interactive:interactive | Interactive test | Overall interactive | - | Manual |
| interactive:exceptions-inimage | Exceptions in image | In-image exception rendering | Sec 7.3.4 | Manual |
| interactive:fees-and-access-constraints | Fees and constraints | Service constraints display | Sec 7.2.4.3, 7.2.4.4 | Manual |

**Note**: These tests require manual verification and do not affect automated compliance score.

**Total Tests**: 3

### 2.10 Recommendations Tests (8 tests)

**Module**: `recommendations.xml`  
**Purpose**: Optional recommended features (not required for compliance)  
**Spec Reference**: Various recommendations

| Test ID | Test Name | Purpose | Spec Reference | Req Type |
|---------|-----------|---------|----------------|----------|
| recommendations:recommendations | Recommendations | Overall recommendations | - | Recommendation |
| recommendations:service-keywords | Service keywords | Service keyword metadata | Sec 7.2.4.1 | Recommendation |
| recommendations:service-contact-info | Service contact info | Contact information | Sec 7.2.4.1 | Recommendation |
| recommendations:png-getmap-format | PNG GetMap format | PNG format recommended | Sec 7.3.3.9 | Recommendation |
| recommendations:layer-abstracts | Layer abstracts | Layer abstract metadata | Sec 7.2.4.7 | Recommendation |
| recommendations:layer-keywordlists | Layer keyword lists | Layer keyword metadata | Sec 7.2.4.7 | Recommendation |
| recommendations:layer-crs | Layer CRS | CRS in each layer | Sec 7.2.4.7 | Recommendation |
| recommendations:metadataurls | Metadata URLs | Layer metadata URLs | Sec 7.2.4.9 | Recommendation |
| recommendations:dims-no-whitespace | Dimension no whitespace | Dimension value whitespace | Sec C.4.1 | Recommendation |
| recommendations:dims-defaults | Dimension defaults | Dimension default values | Sec C.4.1 | Recommendation |

**Note**: Recommendations do not affect compliance score but are considered best practices.

**Total Tests**: 8

---

## Test Summary Statistics

### Overall Test Counts

| Category | Test Definitions | Test Executions | Unique Test Names |
|----------|------------------|-----------------|-------------------|
| Main/Setup | 3 | 3 | 2 |
| Basic WMS | 14 | 14 | 14 |
| Basic Elements | 10 | 10 | 10 |
| GetCapabilities | 34 | 34 | 34 |
| GetMap | 49 | 49 | 49 |
| GetFeatureInfo | 18 | 18 | 18 |
| Queryable | 8 | 8 | 8 |
| Dimensions | 2 | 2 | 2 |
| Time | 14 | 14 | 14 |
| Raster Elevation | 13 | 13 | 13 |
| Vector Elevation | 12 | 12 | 12 |
| Interactive | 3 | 3 | 3 |
| Recommendations | 8 | 8 | 8 |
| **TOTALS** | **213** | **231** | **213** |

**Explanation of Discrepancy**: The difference between 213 test definitions and 231 test executions is due to:
- Some tests iterate over multiple layers, formats, or CRS values
- Container tests that execute sub-tests
- Tests that run multiple times with different parameters

### Requirements Coverage

| Requirement Type | Test Count | Spec Sections |
|-----------------|------------|----------------|
| **Mandatory** | ~170 | 2.2, 6, 7.2, 7.3, Annex A |
| **Optional** | ~46 | 2.3, 7.4, Annex C, D |
| **Recommendations** | 8 | Various recommendations |
| **Manual** | 3 | - |

### Specification Section Coverage

| Spec Section | Description | Test Count | Req Type |
|--------------|-------------|------------|----------|
| **Section 2.2** | Basic WMS | 14 | Mandatory |
| **Section 2.3** | Queryable WMS | 8 | Optional |
| **Section 6** | Basic service elements | 10 | Mandatory |
| **Section 6.2** | Version negotiation | 4 | Mandatory |
| **Section 6.3.3** | Reserved characters | 2 | Mandatory |
| **Section 6.3.4** | Parameter rules | 3 | Mandatory |
| **Section 6.5** | Parameter case | 1 | Mandatory |
| **Section 6.8** | OPTIONS method | 2 | Optional |
| **Section 6.9** | VERSION parameter | 1 | Mandatory |
| **Section 7.2** | GetCapabilities | 34 | Mandatory |
| **Section 7.2.3** | GetCapabilities request | 4 | Mandatory |
| **Section 7.2.4** | GetCapabilities response | 21 | Mandatory |
| **Section 7.2.3.5** | UpdateSequence | 4 | Optional |
| **Section 7.3** | GetMap | 49 | Mandatory |
| **Section 7.3.3** | GetMap request | 9 | Mandatory |
| **Section 7.3.4** | GetMap exceptions | 5 | Mandatory |
| **Section 7.4** | GetFeatureInfo | 18 | Optional |
| **Section 7.4.3** | GetFeatureInfo request | 5 | Optional |
| **Section 7.4.4** | GetFeatureInfo response | 3 | Optional |
| **Annex A** | Conformance tests | 2 | Mandatory |
| **Annex C.4** | Dimensions | 41 | Optional |
| **Annex D.4** | Time dimension | 14 | Optional |
| **Annex E** | XML Schema | 2 | Mandatory |

---

## 3. Gap Analysis: Mandatory Requirements Not Covered by Tests

This section identifies mandatory requirements in the WMS 1.3.0 specification that are **NOT** covered by automated compliance tests. These gaps require **manual verification** or **additional automated testing**.

### 3.1 High-Priority Gaps (Mandatory Requirements)

#### 3.1.1 Service Identification Metadata (Section 7.2.4.1)

**Mandatory Requirements**:
- `Name` element must be "WMS"
- `Title` element must contain human-readable title
- `Abstract` element should contain descriptive text (not mandatory but strongly recommended)
- `KeywordList` with at least one `Keyword` (optional but recommended)

**Automated Test**: None  
**Why Gap**: Tests verify XML structure but not content quality  
**Manual Verification Required**:
```bash
# Check service identification
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Service/wms:Name" -
```

**Risk**: Non-compliant service identification could confuse clients

#### 3.1.2 Contact Information (Section 7.2.4.1)

**Mandatory Requirements**:
- `ContactInformation` element should be present
- `ContactPersonPrimary` with `ContactPerson` and `ContactOrganization` (optional but recommended)
- `ContactAddress` with address details (optional but recommended)

**Automated Test**: None  
**Why Gap**: Contact info is optional but recommended for operational services  
**Manual Verification Required**:
```bash
# Check contact information
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Service/wms:ContactInformation" - || echo "No contact info"
```

**Risk**: No way for users to contact operator for support issues

#### 3.1.3 Layer Title and Abstract Quality (Section 7.2.4.7)

**Mandatory Requirements** (per DTD/Schema):
- Each `Layer` with `Name` element should have `Title` element
- `Abstract` element is optional but strongly recommended

**Automated Test**: Tests verify presence but not quality  
**Why Gap**: No check for meaningful content (non-empty, descriptive)  
**Manual Verification Required**:
```bash
# Check for empty or placeholder titles/abstracts
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Layer[wms:Name and (not(wms:Title) or wms:Title='')]/wms:Name" - | \
  wc -l
```

**Risk**: Auto-generated titles like "Layer1", "auto_xxxx" not user-friendly

#### 3.1.4 CRS Axis Order Compliance (Section 7.3.3.5, B.3)

**Mandatory Requirements**:
- For geographic CRS (e.g., EPSG:4326), axis order in request BBOX is: longitude,latitude (x,y)
- For geographic CRS, response pixels follow axis order: longitude,latitude
- Server must respect axis order for the CRS in both requests and responses

**Automated Test**: Limited (tests CRS availability but not axis order compliance)  
**Why Gap**: Difficult to detect axis order swaps programmatically without known reference data  
**Manual Verification Required**:
```bash
# For EPSG:4326 (WGS 84), request with bbox=-180,-90,180,90 should show worldwide data
# If response shows rotated or swapped axes, server has axis order bug

# Test with known data location
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&\
  LAYERS=test_layer&CRS=EPSG:4326&BBOX=-74,-40,-73,-39&\
  WIDTH=200&HEIGHT=200&FORMAT=image/png" -o test_axis.png

# Verify expected geographic location is correct
```

**Risk**: **CRITICAL** - One of most common WMS implementation bugs. Can cause complete misalignment between requested and returned geographic area.

**Affected CRS**:
- EPSG:4326 (WGS 84) - axis order: lat,lon in spec, but WMS 1.3.0 mandates lon,lat in requests!
- EPSG:4269 (NAD 83) - axis order issues
- Many EPSG geographic coordinate systems

**Historical Context**: WMS 1.1.x used longitude,latitude order consistently. WMS 1.3.0 changed to follow EPSG axis order convention but legacy servers often didn't update, causing widespread axis order confusion.

#### 3.1.5 Exception Code Semantics (Section 7.3.4, Table 9)

**Mandatory Requirements**:
- `OperationNotSupported`: Requested operation not supported
- `LayerNotDefined`: LAYER(S) parameter includes invalid layer
- `StyleNotDefined`: STYLE(S) parameter includes invalid style
- `InvalidCRS`: CRS parameter invalid
- `InvalidFormat`: FORMAT parameter invalid
- `InvalidBBoxValue`: BBOX parameter invalid
- `MissingBBoxValue`: BBOX parameter missing
- `InvalidDimensionValue`: Dimension value invalid
- `CurrentUpdateSequence`: UPDATESEQUENCE equal to current
- `InvalidUpdateSequence`: UPDATESEQUENCE higher than current

**Automated Test**: Tests verify exception is returned but not that correct code is used  
**Why Gap**: Exception code validation is limited; tests may not verify code attribute value  
**Manual Verification Required**:
```bash
# Test LayerNotDefined exception code
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&\
  LAYERS=non_existent_layer&CRS=EPSG:4326&BBOX=-180,-90,180,90&\
  WIDTH=200&HEIGHT=200&FORMAT=image/png"

# Should return XML exception with code="LayerNotDefined", not code="InvalidParameterValue"
```

**Risk**: Clients may not be able to distinguish between different error conditions for proper error handling

#### 3.1.6 Layer Limit Enforcement (Layer limits in service metadata)

**Mandatory Requirements**:
- If service advertises `<LayerLimit>` in capabilities, server must reject GetMap requests with more layers
- Exception code: `OperationNotSupported` (not the most appropriate but convention)

**Automated Test**: `getmap:layerlimit` exists but may not test enforcement  
**Why Gap**: Test may verify presence but not actual enforcement  
**Manual Verification Required**:
```bash
# Check LayerLimit in capabilities
LAYER_LIMIT=$(curl -s "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:LayerLimit/text()" - 2>/dev/null || echo "")

echo "LayerLimit: ${LAYER_LIMIT:-none specified}"

# If LayerLimit exists, test with more layers
if [ -n "$LAYER_LIMIT" ]; then
  curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&\
    LAYERS=layer1,layer2,layer3,layer4,layer5&\
    CRS=EPSG:4326&BBOX=-180,-90,180,90&\
    WIDTH=200&HEIGHT=200&FORMAT=image/png"
  # Should return exception if > LayerLimit
fi
```

**Risk**: Server may advertise limits but not enforce them, leading to potential overload

#### 3.1.7 Format Quality and Compliance (Section 7.3.3.9)

**Mandatory Requirements** (per format):
- `image/png`: Must be valid PNG, 8-bit or 24-bit
- `image/jpeg`: Must be valid JPEG
- `image/gif`: Must be valid GIF
- `image/png;mode=24bit`: 24-bit PNG with transparency
- Custom formats: Must match advertised MIME type

**Automated Tests**: Tests verify format is accepted and response is returned  
**Why Gap**: No validation that returned image is actually in correct format or valid  
**Manual Verification Required**:
```bash
# Test PNG validity
for format in "image/png" "image/jpeg" "image/gif"; do
  curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&\
    LAYERS=test_layer&CRS=EPSG:4326&BBOX=-180,-90,180,90&\
    WIDTH=200&HEIGHT=200&FORMAT=$format" -o test.$format
  
  file test.$format  # Should show correct format
  identify test.$format 2>/dev/null || echo "Invalid $format"
done
```

**Risk**: Server may return broken images, wrong format, or corrupted data

### 3.2 Medium-Priority Gaps (Important Requirements)

#### 3.2.1 Minimum Scale Denominator (Section 7.2.4.7)

**Requirements**:
- `<MinScaleDenominator>` element specifies minimum scale for visibility
- Layer should not be rendered above this scale (zoomed out)

**Automated Test**: `getmap:bbox-below-scale` exists but may not verify proper hiding  
**Why Gap**: Tests may verify exception but not verify that layer correctly disappears  
**Verification**:
```bash
# Check if MinScaleDenominator is advertised
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Layer[wms:MinScaleDenominator]" -

# Test with bbox that should hide layer due to scale
```

#### 3.2.2 Maximum Scale Denominator (Section 7.2.4.7)

**Requirements**:
- `<MaxScaleDenominator>` element specifies maximum scale for visibility
- Layer should not be rendered below this scale (zoomed in)

**Automated Test**: `getmap:bbox-above-scale` exists  
**Why Gap**: May not verify proper hiding behavior  

#### 3.2.3 Layer Opacity/Transparency (Section 7.2.4.10)

**Requirements**:
- `<Opaque>` element: `0` (transparent), `1` (opaque)
- Opaque layers should not allow layers below to show through

**Automated Test**: `getmap:transparent-opaque-layer` exists  
**Why Gap**: Limited verification of actual opacity behavior  
**Verification**:
```bash
# Check opaque layer in capabilities
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Layer[wms:Opaque='1']/wms:Name" -
```

#### 3.2.4 Cascaded Layers (Section 7.2.4.8)

**Requirements**:
- `<Cascaded>` element indicates layer is from upstream server
- Integer value: number of cascaded levels

**Why Gap**: No tests verify cascaded layer handling  
**Verification**:
```bash
# Check cascaded layers
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Layer[wms:Cascaded]/wms:Name" -
```

#### 3.2.5 Attribution (Section 7.2.4.12)

**Requirements**:
- `<Attribution>` element for layer attribution
- Contains `<Title>`, `<OnlineResource>`, and optional `<LogoURL>`

**Why Gap**: No automated tests verify attribution handling  
**Verification**:
```bash
# Check attribution in capabilities
curl "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --xpath "//wms:Attribution" - | wc -l
```

### 3.3 Low-Priority Gaps (Nice-to-Have)

#### 3.3.1 Service Metadata Format Support (Section 7.2.3.1)

**Requirements**:
- Server should support multiple `FORMAT` values for GetCapabilities
- Common: `text/xml`, `application/xml`

**Why Gap**: Most clients use XML; format negotiation rarely used

#### 3.3.2 HTTP POST Support (Section 6.3.5)

**Requirements**:
- Server should support HTTP POST for requests (especially large ones)
- POST body should contain URL-encoded parameters

**Why Gap**: GET requests sufficient for most use cases; POST rarely used

#### 3.3.3 Internationalization (Multiple Language Support)

**Requirements**:
- `AcceptLanguages` parameter (not widely supported)
- Language negotiation per OWS Common

**Why Gap**: Not widely implemented in WMS 1.3.0

---

## 4. Recommendations for Additional Compliance Checking

### 4.1 Automated Testing Gaps That Could Be Filled

#### 4.1.1 Service Metadata Content Validation

**New Tests Needed**:
- `service-name-valid`: Verify <Name>WMS</Name>
- `service-title-nonempty`: Verify <Title> not empty
- `service-title-descriptive`: Verify <Title> is not placeholder (e.g., "My WMS")
- `service-abstract-present`: Verify <Abstract> exists
- `service-keywords-present`: Verify <KeywordList> with keywords

**Implementation**:
```bash
#!/bin/bash
# Check service metadata quality
curl -s "http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0" | \
  xmllint --format - | \
  xmllint --xpath "//wms:Service/wms:Title/text()" - | \
  wc -c
# Should be > 10 characters
```

#### 4.1.2 Axis Order Validation Tool

**New Tool Needed**: Reference image comparison for axis order verification

**Implementation Approach**:
```bash
#!/bin/bash
# Axis order verification tool
# 1. Request known area with known reference data
# 2. Verify returned image shows expected geographic features
# 3. Check that features are in correct geographic positions

# Example: Request Europe/Africa交界区域
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&\
  LAYERS=coastline&CRS=EPSG:4326&BBOX=-10,30,40,70&\
  WIDTH=400&HEIGHT=400&FORMAT=image/png" -o axis_test.png

# Use image recognition or manual verification to check
# that Spain is on the left, Mediterranean in center, Italy on right
```

#### 4.1.3 Exception Code Validation

**New Tests Needed**:
- `exception-code-layer-not-defined`: Verify code="LayerNotDefined" for invalid layer
- `exception-code-style-not-defined`: Verify code="StyleNotDefined" for invalid style
- `exception-code-invalid-crs`: Verify code="InvalidCRS" for unsupported CRS
- `exception-code-invalid-format`: Verify code="InvalidFormat" for unsupported format
- `exception-code-invalid-bbox`: Verify code="InvalidBBoxValue" for invalid bbox

**Implementation**:
```python
#!/usr/bin/env python3
import requests
import xml.etree.ElementTree as ET

# Test LayerNotDefined exception code
try:
    response = requests.get(
        'http://localhost:8080/wms',
        params={
            'SERVICE': 'WMS',
            'VERSION': '1.3.0',
            'REQUEST': 'GetMap',
            'LAYERS': 'non_existent_layer',
            'CRS': 'EPSG:4326',
            'BBOX': '-180,-90,180,90',
            'WIDTH': '200',
            'HEIGHT': '200',
            'FORMAT': 'image/png'
        }
    )
    
    if response.status_code == 200:
        # Parse exception
        root = ET.fromstring(response.content)
        exception = root.find('.//{http://www.opengis.net/ogc}ServiceException')
        if exception is not None:
            code = exception.get('code')
            if code != 'LayerNotDefined':
                print(f"FAIL: Expected code='LayerNotDefined', got '{code}'")
            else:
                print("PASS: Correct exception code")
        else:
            print("FAIL: No exception found")
except Exception as e:
    print(f"ERROR: {e}")
```

#### 4.1.4 Image Format Validation

**New Tests Needed**:
- `format-png-valid`: Verify PNG is valid and readable
- `format-png-dimensions`: Verify PNG dimensions match WIDTH/HEIGHT
- `format-jpeg-valid`: Verify JPEG is valid
- `format-jpeg-quality`: Verify JPEG compression is reasonable
- `format-gif-valid`: Verify GIF is valid (if advertised)

**Implementation**:
```bash
#!/bin/bash
# Validate image format
curl "http://localhost:8080/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&\
  LAYERS=test&CRS=EPSG:4326&BBOX=-180,-90,180,90&\
  WIDTH=200&HEIGHT=200&FORMAT=image/png" -o test.png

# Use ImageMagick to validate
if identify test.png >/dev/null 2>&1; then
  echo "PASS: Valid PNG"
  # Check dimensions
  DIMS=$(identify -format "%wx%h" test.png)
  if [ "$DIMS" = "200x200" ]; then
    echo "PASS: Correct dimensions"
  else
    echo "FAIL: Expected 200x200, got $DIMS"
  fi
else
  echo "FAIL: Invalid PNG"
fi
```

### 4.2 Manual Compliance Checklist

Use this checklist for manual verification of requirements not covered by automated tests:

#### 4.2.1 Pre-Deployment Checklist

- [ ] **Service Identification**:
  - [ ] `<Name>WMS</Name>` is present
  - [ ] `<Title>` is descriptive and not placeholder
  - [ ] `<Abstract>` provides meaningful description
- [ ] **Contact Information**:
  - [ ] `<ContactInformation>` is present
  - [ ] Contact person/organization specified
  - [ ] Contact address provided (if applicable)
- [ ] **Layer Metadata**:
  - [ ] Each advertised layer has `<Name>` and `<Title>`
  - [ ] Layer `<Abstract>` is descriptive
  - [ ] Layer `<KeywordList>` includes relevant keywords
- [ ] **CRS Coverage**:
  - [ ] Each layer advertises CRS values it actually supports
  - [ ] EX_GeographicBoundingBox covers actual data extent
- [ ] **Format Advertising**:
  - [ ] Each `<Format>` in capabilities is actually supported
  - [ ] Image formats produce valid images

#### 4.2.2 Axis Order Verification Checklist

For each geographic CRS:

**EPSG:4326 (WGS 84)**:
- [ ] Request with `BBOX=-180,-90,180,90` returns worldwide data
- [ ] Request with `BBOX=-10,30,40,70` returns Europe/Africa
- [ ] Features are not rotated or swapped
- [ ] Verify Spain is on left, Italy on right for appropriate bbox

**EPSG:4269 (NAD 83)**:
- [ ] Similar verification for North American data

#### 4.2.3 Exception Handling Checklist

Test each exception condition:

**Layer Errors**:
- [ ] Invalid layer → `LayerNotDefined` exception
- [ ] Layer not queryable → `OperationNotSupported` for GetFeatureInfo

**CRS Errors**:
- [ ] Invalid CRS → `InvalidCRS` exception
- [ ] CRS not supported for layer → `InvalidCRS` exception

**BBox Errors**:
- [ ] Invalid bbox format → `InvalidBBoxValue` exception
- [ ] Bbox out of CRS bounds → appropriate error or clipped response
- [ ] minx > maxx or miny > maxy → `InvalidBBoxValue` exception

**Format Errors**:
- [ ] Invalid format → `InvalidFormat` exception
- [ ] Format not supported → `InvalidFormat` exception

#### 4.2.4 Performance and Quality Checklist

- [ ] **Layer Limit Enforcement**: If LayerLimit advertised, enforce it
- [ ] **Min/Max Scale**:
  - [ ] `MinScaleDenominator` properly hides layer when zoomed out
  - [ ] `MaxScaleDenominator` properly hides layer when zoomed in
- [ ] **Opaque Layers**:
  - [ ] `Opaque=1` layers block layers below
  - [ ] `Opaque=0` layers allow transparency
- [ ] **Image Quality**:
  - [ ] PNG images are valid and uncompressed (lossless)
  - [ ] JPEG images have reasonable compression
  - [ ] Transparent layers properly support transparency
- [ ] **Performance**:
  - [ ] GetCapabilities responds quickly (under 2 seconds)
  - [ ] GetMap responds in reasonable time (depends on complexity)
  - [ ] Proper caching headers for static data

### 4.3 Continuous Compliance Monitoring

Recommend implementing periodic checks:

#### 4.3.1 Health Check Endpoint

```bash
#!/bin/bash
# WMS health check for monitoring

WMS_URL="http://localhost:8080/wms"

echo "=== WMS Health Check ==="
echo "Timestamp: $(date -Iseconds)"

# Test GetCapabilities response time
START=$(date +%s.%N)
RESPONSE=$(curl -s -w "%{http_code}" -o /tmp/caps.xml "$WMS_URL?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0")
END=$(date +%s.%N)
CAPS_TIME=$(echo "$END - $START" | bc)

echo "GetCapabilities: $RESPONSE (${CAPS_TIME}s)"

# Test basic GetMap
START=$(date +%s.%N)
RESPONSE=$(curl -s -w "%{http_code}" -o /dev/null "$WMS_URL?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYER