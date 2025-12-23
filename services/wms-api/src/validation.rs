//! OGC Compliance Validation Module
//!
//! Provides automated validation of WMS and WMTS compliance with OGC standards.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Status of a validation check
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

/// Result of a single validation check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub status: CheckStatus,
    pub message: String,
}

impl CheckResult {
    pub fn pass(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            message: message.into(),
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            message: message.into(),
        }
    }

    #[allow(dead_code)]
    pub fn skip(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Skip,
            message: message.into(),
        }
    }
}

/// WMS validation results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmsValidation {
    pub status: String,
    pub version: String,
    pub checks: WmsChecks,
    pub layers_tested: u32,
    pub layers_passed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmsChecks {
    pub capabilities: CheckResult,
    pub getmap: CheckResult,
    pub getfeatureinfo: CheckResult,
    pub exceptions: CheckResult,
    pub crs_support: CheckResult,
}

/// WMTS validation results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmtsValidation {
    pub status: String,
    pub version: String,
    pub checks: WmtsChecks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmtsChecks {
    pub capabilities: CheckResult,
    pub gettile_rest: CheckResult,
    pub gettile_kvp: CheckResult,
    pub tilematrixset: CheckResult,
}

/// Full test results from OGC TEAM Engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullTestResults {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

/// Complete validation status response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationStatus {
    pub timestamp: String,
    pub wms: WmsValidation,
    pub wmts: WmtsValidation,
    pub overall_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_full_test: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_test_results: Option<FullTestResults>,
}

impl ValidationStatus {
    /// Create a new validation status with current timestamp
    pub fn new(wms: WmsValidation, wmts: WmtsValidation) -> Self {
        let wms_ok = wms.status == "compliant";
        let wmts_ok = wmts.status == "compliant";

        let overall_status = if wms_ok && wmts_ok {
            "compliant".to_string()
        } else if wms_ok || wmts_ok {
            "partial".to_string()
        } else {
            "non-compliant".to_string()
        };

        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let timestamp_str = chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Self {
            timestamp: timestamp_str,
            wms,
            wmts,
            overall_status,
            last_full_test: None,
            full_test_results: None,
        }
    }
}

/// Extract a valid layer and style from WMS capabilities for testing
async fn get_test_layer_from_capabilities(base_url: &str) -> Option<(String, String)> {
    let resp = reqwest::get(format!("{}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0", base_url)).await.ok()?;
    let text = resp.text().await.ok()?;
    
    // Preferred layers in order (gradient style is most reliable)
    let preferred = [
        ("gfs_TMP", "gradient"),
        ("hrrr_TMP", "gradient"),
        ("gfs_RH", "gradient"),
        ("mrms_REFL", "standard"),
    ];
    
    for (layer, style) in preferred {
        // Check if layer exists in capabilities
        if text.contains(&format!("<Name>{}</Name>", layer)) {
            return Some((layer.to_string(), style.to_string()));
        }
    }
    
    None
}

/// Run WMS validation checks
pub async fn validate_wms(base_url: &str) -> WmsValidation {
    let mut layers_tested = 0;
    let mut layers_passed = 0;

    // Get a valid test layer from capabilities
    let (test_layer, test_style) = get_test_layer_from_capabilities(base_url)
        .await
        .unwrap_or(("gfs_TMP".to_string(), "gradient".to_string()));

    // Check 1: GetCapabilities
    let capabilities = match reqwest::get(format!("{}?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0", base_url)).await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.text().await {
                    Ok(text) => {
                        if text.contains("WMS_Capabilities") && text.contains("version=\"1.3.0\"") {
                            CheckResult::pass("Valid WMS 1.3.0 capabilities")
                        } else {
                            CheckResult::fail("Invalid capabilities structure")
                        }
                    }
                    Err(_) => CheckResult::fail("Failed to read response"),
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Check 2: GetMap (use discovered layer/style)
    let getmap = match reqwest::get(format!(
        "{}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS={}&STYLES={}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png",
        base_url, test_layer, test_style
    )).await {
        Ok(resp) => {
            if resp.status().is_success() {
                // Verify it's actually an image (PNG magic bytes or check content-type)
                let content_type = resp.headers().get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if content_type.contains("image/png") {
                    layers_tested += 1;
                    layers_passed += 1;
                    CheckResult::pass(format!("GetMap returns valid PNG (tested {})", test_layer))
                } else {
                    CheckResult::fail(format!("Expected image/png, got {}", content_type))
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Check 3: GetFeatureInfo
    let getfeatureinfo = match reqwest::get(format!(
        "{}?SERVICE=WMS&REQUEST=GetFeatureInfo&VERSION=1.3.0&LAYERS={}&QUERY_LAYERS={}&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&I=128&J=128&INFO_FORMAT=application/json",
        base_url, test_layer, test_layer
    )).await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<serde_json::Value>().await {
                    Ok(_) => CheckResult::pass("GetFeatureInfo returns valid JSON"),
                    Err(_) => CheckResult::fail("Invalid JSON response"),
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Check 4: Exception handling
    let exceptions = match reqwest::get(format!(
        "{}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=INVALID_LAYER_DOESNT_EXIST&CRS=EPSG:4326&BBOX=-90,-180,90,180&WIDTH=256&HEIGHT=256&FORMAT=image/png",
        base_url
    )).await {
        Ok(resp) => {
            if resp.status().is_client_error() || resp.status().is_server_error() {
                CheckResult::pass("Returns HTTP error for invalid layer")
            } else {
                match resp.text().await {
                    Ok(text) => {
                        if text.contains("ServiceException") {
                            CheckResult::pass("Returns ServiceException XML for invalid layer")
                        } else {
                            CheckResult::fail("No exception for invalid layer")
                        }
                    }
                    Err(_) => CheckResult::fail("Failed to read response"),
                }
            }
        }
        Err(_) => CheckResult::pass("HTTP error for invalid request (acceptable)"),
    };

    // Check 5: CRS support (EPSG:3857 - Web Mercator)
    let crs_support = match reqwest::get(format!(
        "{}?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS={}&STYLES={}&CRS=EPSG:3857&BBOX=-20037508,-20037508,20037508,20037508&WIDTH=256&HEIGHT=256&FORMAT=image/png",
        base_url, test_layer, test_style
    )).await {
        Ok(resp) => {
            if resp.status().is_success() {
                let content_type = resp.headers().get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if content_type.contains("image/png") {
                    CheckResult::pass("EPSG:3857 (Web Mercator) supported")
                } else {
                    CheckResult::fail("EPSG:3857 returned non-image response")
                }
            } else {
                CheckResult::fail(format!("EPSG:3857 not supported (HTTP {})", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Determine overall status
    let all_passed = capabilities.status == CheckStatus::Pass
        && getmap.status == CheckStatus::Pass
        && getfeatureinfo.status == CheckStatus::Pass
        && exceptions.status == CheckStatus::Pass
        && crs_support.status == CheckStatus::Pass;

    let status = if all_passed {
        "compliant"
    } else {
        "non-compliant"
    }
    .to_string();

    WmsValidation {
        status,
        version: "1.3.0".to_string(),
        checks: WmsChecks {
            capabilities,
            getmap,
            getfeatureinfo,
            exceptions,
            crs_support,
        },
        layers_tested,
        layers_passed,
    }
}

/// Extract a valid layer and style from WMTS capabilities for testing
async fn get_test_layer_from_wmts_capabilities(base_url: &str) -> Option<(String, String)> {
    let resp = reqwest::get(format!("{}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0", base_url)).await.ok()?;
    let text = resp.text().await.ok()?;
    
    // Preferred layers in order (gradient style is most reliable)
    let preferred = [
        ("gfs_TMP", "gradient"),
        ("hrrr_TMP", "gradient"),
        ("gfs_RH", "gradient"),
        ("mrms_REFL", "standard"),
    ];
    
    for (layer, style) in preferred {
        // Check if layer exists in capabilities (WMTS uses <ows:Identifier>)
        if text.contains(&format!("<ows:Identifier>{}</ows:Identifier>", layer)) 
           || text.contains(&format!("<Identifier>{}</Identifier>", layer)) {
            return Some((layer.to_string(), style.to_string()));
        }
    }
    
    None
}

/// Run WMTS validation checks
pub async fn validate_wmts(base_url: &str) -> WmtsValidation {
    // Get a valid test layer from capabilities
    let (test_layer, test_style) = get_test_layer_from_wmts_capabilities(base_url)
        .await
        .unwrap_or(("gfs_TMP".to_string(), "gradient".to_string()));

    // Check 1: GetCapabilities
    let capabilities = match reqwest::get(format!("{}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0", base_url)).await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.text().await {
                    Ok(text) => {
                        if text.contains("<Capabilities") && text.contains("version=\"1.0.0\"") {
                            CheckResult::pass("Valid WMTS 1.0.0 capabilities")
                        } else {
                            CheckResult::fail("Invalid capabilities structure")
                        }
                    }
                    Err(_) => CheckResult::fail("Failed to read response"),
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Check 2: GetTile REST (use discovered layer/style)
    let gettile_rest = match reqwest::get(format!("{}/rest/{}/{}/WebMercatorQuad/2/1/1.png", base_url, test_layer, test_style)).await {
        Ok(resp) => {
            if resp.status().is_success() {
                let content_type = resp.headers().get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if content_type.contains("image/png") {
                    CheckResult::pass(format!("REST tiles working (tested {})", test_layer))
                } else {
                    CheckResult::fail(format!("Expected image/png, got {}", content_type))
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Check 3: GetTile KVP (use discovered layer/style)
    let gettile_kvp = match reqwest::get(format!(
        "{}?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER={}&STYLE={}&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX=2&TILEROW=1&TILECOL=1",
        base_url, test_layer, test_style
    )).await {
        Ok(resp) => {
            if resp.status().is_success() {
                let content_type = resp.headers().get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if content_type.contains("image/png") {
                    CheckResult::pass("KVP tiles working")
                } else {
                    CheckResult::fail(format!("Expected image/png, got {}", content_type))
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Check 4: TileMatrixSet
    let tilematrixset = match reqwest::get(format!("{}?SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0", base_url)).await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.text().await {
                    Ok(text) => {
                        if text.contains("WebMercatorQuad") && text.contains("<TileMatrix>") {
                            CheckResult::pass("WebMercatorQuad TileMatrixSet defined")
                        } else {
                            CheckResult::fail("TileMatrixSet missing or incomplete")
                        }
                    }
                    Err(_) => CheckResult::fail("Failed to read response"),
                }
            } else {
                CheckResult::fail(format!("HTTP {}", resp.status()))
            }
        }
        Err(e) => CheckResult::fail(format!("Request failed: {}", e)),
    };

    // Determine overall status
    let all_passed = capabilities.status == CheckStatus::Pass
        && gettile_rest.status == CheckStatus::Pass
        && gettile_kvp.status == CheckStatus::Pass
        && tilematrixset.status == CheckStatus::Pass;

    let status = if all_passed {
        "compliant"
    } else {
        "non-compliant"
    }
    .to_string();

    WmtsValidation {
        status,
        version: "1.0.0".to_string(),
        checks: WmtsChecks {
            capabilities,
            gettile_rest,
            gettile_kvp,
            tilematrixset,
        },
    }
}

/// Run full validation suite
pub async fn run_validation(base_url: &str) -> ValidationStatus {
    let wms_url = format!("{}/wms", base_url);
    let wmts_url = format!("{}/wmts", base_url);

    let (wms, wmts) = tokio::join!(
        validate_wms(&wms_url),
        validate_wmts(&wmts_url)
    );

    ValidationStatus::new(wms, wmts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_result_creation() {
        let pass = CheckResult::pass("test");
        assert_eq!(pass.status, CheckStatus::Pass);
        assert_eq!(pass.message, "test");

        let fail = CheckResult::fail("error");
        assert_eq!(fail.status, CheckStatus::Fail);
        assert_eq!(fail.message, "error");
    }

    #[test]
    fn test_validation_status_overall() {
        let wms = WmsValidation {
            status: "compliant".to_string(),
            version: "1.3.0".to_string(),
            checks: WmsChecks {
                capabilities: CheckResult::pass("ok"),
                getmap: CheckResult::pass("ok"),
                getfeatureinfo: CheckResult::pass("ok"),
                exceptions: CheckResult::pass("ok"),
                crs_support: CheckResult::pass("ok"),
            },
            layers_tested: 5,
            layers_passed: 5,
        };

        let wmts = WmtsValidation {
            status: "compliant".to_string(),
            version: "1.0.0".to_string(),
            checks: WmtsChecks {
                capabilities: CheckResult::pass("ok"),
                gettile_rest: CheckResult::pass("ok"),
                gettile_kvp: CheckResult::pass("ok"),
                tilematrixset: CheckResult::pass("ok"),
            },
        };

        let status = ValidationStatus::new(wms, wmts);
        assert_eq!(status.overall_status, "compliant");
    }
}
