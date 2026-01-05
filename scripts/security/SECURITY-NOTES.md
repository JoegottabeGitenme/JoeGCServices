# Security Scan Notes & False Positive Documentation

This document explains known false positives and intentional security configurations
that may be flagged by automated security scanners.

## Last Reviewed: 2026-01-05

---

## Intentional Security Configurations

### 1. CORS: `Access-Control-Allow-Origin: *`

**Status:** Intentional - Required for OGC API compatibility

**Endpoints affected:** `/wms`, `/wmts`, `/edr/*`

**Reason:** OGC web mapping APIs (WMS, WMTS, EDR) must be accessible from any origin.
Web mapping applications like OpenLayers, Leaflet, and other GIS clients need to
fetch map tiles and data from these endpoints. Restricting CORS would break legitimate
use cases.

**Mitigation:**
- Rate limiting is applied to prevent abuse
- No authentication/session data is exposed via these endpoints
- These endpoints only return public weather data

**Scanner flags this as:**
- ZAP: "CORS Misconfiguration" (Medium)
- ZAP: "Cross-Domain Misconfiguration" (Medium)

---

### 2. CSP: `script-src 'unsafe-inline'`

**Status:** Intentional - Required for admin dashboard functionality

**Endpoints affected:** Admin dashboard HTML pages

**Reason:** The admin dashboard uses inline scripts for interactive features.
Refactoring to remove all inline scripts would require significant effort for
pages that are already protected by HTTP Basic Auth.

**Mitigation:**
- Admin pages require authentication
- Only affects HTML pages, not API endpoints
- API responses return JSON and don't execute scripts

**Scanner flags this as:**
- ZAP: "CSP: script-src unsafe-inline" (Medium)

---

### 3. CSP: `style-src 'unsafe-inline'`

**Status:** Intentional - Required for dashboard styling

**Endpoints affected:** Admin dashboard HTML pages

**Reason:** Admin dashboard pages use inline styles for layout and UI.

**Mitigation:**
- Same as script-src - protected by authentication
- Only affects HTML pages

**Scanner flags this as:**
- ZAP: "CSP: style-src unsafe-inline" (Medium)

---

### 4. CSP: Wildcard in `img-src` and `connect-src`

**Status:** Intentional - Required for external resources

**Configuration:**
- `img-src 'self' data: https:`
- `connect-src 'self' https:`

**Reason:**
- `img-src https:` allows loading map tiles and images from any HTTPS source
- `connect-src https:` allows API calls to external services (Cloudflare analytics, etc.)

**Scanner flags this as:**
- ZAP: "CSP: Wildcard Directive" (Medium)

---

### 5. Sub Resource Integrity (SRI) Missing

**Status:** Acknowledged - Low risk for our use case

**Endpoints affected:** Pages loading external scripts (Redoc, Cloudflare Insights)

**Reason:** External scripts are loaded from trusted CDNs. Adding SRI would require
updating hashes whenever these scripts are updated by their providers.

**Mitigation:**
- External scripts are loaded over HTTPS only
- Scripts are from trusted providers (Cloudflare, Redoc)

**Scanner flags this as:**
- ZAP: "Sub Resource Integrity Attribute Missing" (Medium)

---

## Known False Positives

### 1. "Source Code Disclosure - SQL"

**Finding:** ZAP flagged `edr-coverage.html` for SQL disclosure

**Actual content:** UI placeholder text "Select an item from the tree to see details"

**Analysis:** The word "Select" triggered ZAP's SQL detection heuristic.
This is not SQL code - it's just English text describing UI behavior.

**Status:** FALSE POSITIVE - No action needed

---

### 2. "Path Traversal" on WMS endpoints

**Finding:** ZAP flagged WMS query parameters as path traversal

**Actual content:** Standard OGC WMS parameters like `SERVICE=wms`, `VERSION=1.3.0`

**Analysis:** ZAP incorrectly interpreted OGC parameter syntax as path traversal
attempts. WMS/WMTS use query string parameters with specific syntax that differs
from typical web applications.

**Status:** FALSE POSITIVE - No action needed

---

### 3. "Source Code Disclosure - File Inclusion" on WMS endpoints

**Finding:** ZAP flagged WMS endpoints for file inclusion

**Analysis:** Same as path traversal - OGC parameters are being misinterpreted.

**Status:** FALSE POSITIVE - No action needed

---

### 4. Nuclei SSRF on `/grafana/edr/...` paths

**Finding:** Nuclei templates flagged SSRF on paths like `/grafana/edr/collections/...`

**Analysis:** The Nuclei templates were incorrectly targeting Grafana's 404 page
instead of the actual EDR API. The templates used `{{BaseURL}}` with full endpoint
URLs, causing path duplication.

**Fix applied:** Updated `nuclei-scan.sh` to use base URL only for OGC templates.

**Status:** FIXED in commit after 2026-01-04

---

## Security Headers Summary

Current security headers configured in nginx:

| Header | Value | Purpose |
|--------|-------|---------|
| Strict-Transport-Security | max-age=31536000; includeSubDomains | Force HTTPS |
| X-Content-Type-Options | nosniff | Prevent MIME sniffing |
| X-Frame-Options | SAMEORIGIN | Clickjacking protection |
| Referrer-Policy | strict-origin-when-cross-origin | Control referrer leakage |
| Permissions-Policy | (disabled features) | Disable unused APIs |
| Cross-Origin-Embedder-Policy | unsafe-none | Spectre mitigation |
| Cross-Origin-Opener-Policy | same-origin-allow-popups | Spectre mitigation |
| Cross-Origin-Resource-Policy | cross-origin | Allow cross-origin resource sharing |

---

## Recommendations for Future Scans

1. **Exclude `/grafana/` paths** from OGC-specific templates - Grafana has its own security model
2. **Configure ZAP context** to understand OGC parameter syntax and avoid false positives
3. **Review SRI** if external scripts cause issues - consider pinning versions with hashes
4. **Consider API-specific CORS** if abuse becomes an issue - can restrict to known domains

---

## Contact

For security concerns, contact the repository maintainers.
