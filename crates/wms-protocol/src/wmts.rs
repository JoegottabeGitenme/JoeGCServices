//! OGC WMTS protocol implementation.
//!
//! Supports WMTS 1.0.0 specification with both KVP and RESTful bindings.
// TODO ask claude if this file is used
use serde::Deserialize;

use wms_common::{BoundingBox, CrsCode, TileCoord, TileMatrixSet, WmsError, WmsResult};

/// WMTS request types.
#[derive(Debug, Clone)]
pub enum WmtsRequest {
    GetCapabilities(GetCapabilitiesRequest),
    GetTile(GetTileRequest),
    GetFeatureInfo(GetFeatureInfoRequest),
}

/// GetCapabilities request parameters.
#[derive(Debug, Clone, Default)]
pub struct GetCapabilitiesRequest {
    pub version: Option<String>,
    pub sections: Option<Vec<String>>,
}

/// GetTile request parameters.
#[derive(Debug, Clone)]
pub struct GetTileRequest {
    /// Layer identifier
    pub layer: String,

    /// Style identifier
    pub style: String,

    /// Output format (e.g., "image/png")
    pub format: String,

    /// TileMatrixSet identifier
    pub tile_matrix_set: String,

    /// TileMatrix (zoom level) identifier
    pub tile_matrix: String,

    /// Tile row
    pub tile_row: u32,

    /// Tile column
    pub tile_col: u32,

    /// Optional time dimension
    pub time: Option<String>,

    /// Optional elevation dimension
    pub elevation: Option<String>,

    /// Other dimensions
    pub dimensions: std::collections::HashMap<String, String>,
}

impl GetTileRequest {
    /// Convert to TileCoord.
    pub fn to_tile_coord(&self) -> WmsResult<TileCoord> {
        let z: u32 = self
            .tile_matrix
            .parse()
            .map_err(|_| WmsError::InvalidParameter {
                param: "TileMatrix".to_string(),
                message: format!("Invalid zoom level: {}", self.tile_matrix),
            })?;

        Ok(TileCoord {
            z,
            x: self.tile_col,
            y: self.tile_row,
        })
    }

    /// Generate cache key for this request.
    pub fn cache_key(&self) -> String {
        let time_part = self.time.as_deref().unwrap_or("default");
        format!(
            "wmts:{}:{}:{}:{}:{}:{}:{}",
            self.layer,
            self.style,
            self.tile_matrix_set,
            self.tile_matrix,
            self.tile_row,
            self.tile_col,
            time_part
        )
    }
}

/// GetFeatureInfo request parameters.
#[derive(Debug, Clone)]
pub struct GetFeatureInfoRequest {
    pub layer: String,
    pub style: String,
    pub format: String,
    pub tile_matrix_set: String,
    pub tile_matrix: String,
    pub tile_row: u32,
    pub tile_col: u32,
    pub i: u32,
    pub j: u32,
    pub info_format: String,
}

/// KVP (Key-Value Pair) query string parameters for WMTS.
#[derive(Debug, Deserialize)]
pub struct WmtsKvpParams {
    #[serde(rename = "SERVICE")]
    pub service: Option<String>,

    #[serde(rename = "REQUEST")]
    pub request: Option<String>,

    #[serde(rename = "VERSION")]
    pub version: Option<String>,

    // GetTile params
    #[serde(rename = "LAYER")]
    pub layer: Option<String>,

    #[serde(rename = "STYLE")]
    pub style: Option<String>,

    #[serde(rename = "FORMAT")]
    pub format: Option<String>,

    #[serde(rename = "TILEMATRIXSET")]
    pub tile_matrix_set: Option<String>,

    #[serde(rename = "TILEMATRIX")]
    pub tile_matrix: Option<String>,

    #[serde(rename = "TILEROW")]
    pub tile_row: Option<u32>,

    #[serde(rename = "TILECOL")]
    pub tile_col: Option<u32>,

    #[serde(rename = "TIME")]
    pub time: Option<String>,

    // GetFeatureInfo params
    #[serde(rename = "I")]
    pub i: Option<u32>,

    #[serde(rename = "J")]
    pub j: Option<u32>,

    #[serde(rename = "INFOFORMAT")]
    pub info_format: Option<String>,
}

impl WmtsKvpParams {
    /// Parse into a typed request.
    pub fn into_request(self) -> WmsResult<WmtsRequest> {
        // Validate SERVICE
        if self.service.as_deref() != Some("WMTS") {
            return Err(WmsError::InvalidParameter {
                param: "SERVICE".to_string(),
                message: "SERVICE must be WMTS".to_string(),
            });
        }

        match self.request.as_deref() {
            Some("GetCapabilities") => Ok(WmtsRequest::GetCapabilities(GetCapabilitiesRequest {
                version: self.version,
                sections: None,
            })),
            Some("GetTile") => {
                let layer = self
                    .layer
                    .ok_or_else(|| WmsError::MissingParameter("LAYER".to_string()))?;
                let style = self.style.unwrap_or_else(|| "default".to_string());
                let format = self.format.unwrap_or_else(|| "image/png".to_string());
                let tile_matrix_set = self
                    .tile_matrix_set
                    .ok_or_else(|| WmsError::MissingParameter("TILEMATRIXSET".to_string()))?;
                let tile_matrix = self
                    .tile_matrix
                    .ok_or_else(|| WmsError::MissingParameter("TILEMATRIX".to_string()))?;
                let tile_row = self
                    .tile_row
                    .ok_or_else(|| WmsError::MissingParameter("TILEROW".to_string()))?;
                let tile_col = self
                    .tile_col
                    .ok_or_else(|| WmsError::MissingParameter("TILECOL".to_string()))?;

                Ok(WmtsRequest::GetTile(GetTileRequest {
                    layer,
                    style,
                    format,
                    tile_matrix_set,
                    tile_matrix,
                    tile_row,
                    tile_col,
                    time: self.time,
                    elevation: None,
                    dimensions: std::collections::HashMap::new(),
                }))
            }
            Some("GetFeatureInfo") => {
                let layer = self
                    .layer
                    .ok_or_else(|| WmsError::MissingParameter("LAYER".to_string()))?;
                let style = self.style.unwrap_or_else(|| "default".to_string());
                let format = self.format.unwrap_or_else(|| "image/png".to_string());
                let tile_matrix_set = self
                    .tile_matrix_set
                    .ok_or_else(|| WmsError::MissingParameter("TILEMATRIXSET".to_string()))?;
                let tile_matrix = self
                    .tile_matrix
                    .ok_or_else(|| WmsError::MissingParameter("TILEMATRIX".to_string()))?;
                let tile_row = self
                    .tile_row
                    .ok_or_else(|| WmsError::MissingParameter("TILEROW".to_string()))?;
                let tile_col = self
                    .tile_col
                    .ok_or_else(|| WmsError::MissingParameter("TILECOL".to_string()))?;
                let i = self
                    .i
                    .ok_or_else(|| WmsError::MissingParameter("I".to_string()))?;
                let j = self
                    .j
                    .ok_or_else(|| WmsError::MissingParameter("J".to_string()))?;
                let info_format = self.info_format.unwrap_or_else(|| "text/plain".to_string());

                Ok(WmtsRequest::GetFeatureInfo(GetFeatureInfoRequest {
                    layer,
                    style,
                    format,
                    tile_matrix_set,
                    tile_matrix,
                    tile_row,
                    tile_col,
                    i,
                    j,
                    info_format,
                }))
            }
            Some(req) => Err(WmsError::InvalidParameter {
                param: "REQUEST".to_string(),
                message: format!("Unknown request: {}", req),
            }),
            None => Err(WmsError::MissingParameter("REQUEST".to_string())),
        }
    }
}

/// RESTful URL path parameters for WMTS.
#[derive(Debug, Clone)]
pub struct WmtsRestPath {
    pub layer: String,
    pub style: String,
    pub tile_matrix_set: String,
    pub tile_matrix: String,
    pub tile_row: u32,
    pub tile_col: u32,
    pub format: String,
    pub time: Option<String>,
}

impl WmtsRestPath {
    /// Parse a RESTful WMTS URL path.
    ///
    /// Expected formats:
    /// - /{layer}/{style}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.{format}
    /// - /{layer}/{style}/{time}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.{format}
    pub fn parse(path: &str) -> WmsResult<Self> {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();

        if parts.len() < 6 {
            return Err(WmsError::InvalidParameter {
                param: "path".to_string(),
                message: "Invalid RESTful path format".to_string(),
            });
        }

        // Handle both with and without time dimension
        let (layer, style, time, tms_idx) = if parts.len() >= 7 {
            // With time: layer/style/time/TileMatrixSet/...
            (parts[0], parts[1], Some(parts[2].to_string()), 3)
        } else {
            // Without time: layer/style/TileMatrixSet/...
            (parts[0], parts[1], None, 2)
        };

        let tile_matrix_set = parts[tms_idx];
        let tile_matrix = parts[tms_idx + 1];
        let tile_row: u32 = parts[tms_idx + 2]
            .parse()
            .map_err(|_| WmsError::InvalidParameter {
                param: "TileRow".to_string(),
                message: "Invalid tile row".to_string(),
            })?;

        // Last part is TileCol.format
        let last = parts[tms_idx + 3];
        let (tile_col_str, format) =
            last.rsplit_once('.')
                .ok_or_else(|| WmsError::InvalidParameter {
                    param: "TileCol".to_string(),
                    message: "Missing format extension".to_string(),
                })?;

        let tile_col: u32 = tile_col_str
            .parse()
            .map_err(|_| WmsError::InvalidParameter {
                param: "TileCol".to_string(),
                message: "Invalid tile column".to_string(),
            })?;

        let format = match format {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => format,
        };

        Ok(Self {
            layer: layer.to_string(),
            style: style.to_string(),
            tile_matrix_set: tile_matrix_set.to_string(),
            tile_matrix: tile_matrix.to_string(),
            tile_row,
            tile_col,
            format: format.to_string(),
            time,
        })
    }

    /// Convert to GetTileRequest.
    pub fn into_request(self) -> GetTileRequest {
        GetTileRequest {
            layer: self.layer,
            style: self.style,
            format: self.format,
            tile_matrix_set: self.tile_matrix_set,
            tile_matrix: self.tile_matrix,
            tile_row: self.tile_row,
            tile_col: self.tile_col,
            time: self.time,
            elevation: None,
            dimensions: std::collections::HashMap::new(),
        }
    }
}

/// Generate WMTS Capabilities XML document.
pub struct WmtsCapabilitiesBuilder {
    pub service_title: String,
    pub service_abstract: String,
    pub service_url: String,
    pub layers: Vec<WmtsLayerInfo>,
    pub tile_matrix_sets: Vec<TileMatrixSet>,
}

/// Layer information for capabilities.
#[derive(Debug, Clone)]
pub struct WmtsLayerInfo {
    pub identifier: String,
    pub title: String,
    pub abstract_text: Option<String>,
    pub styles: Vec<WmtsStyleInfo>,
    pub formats: Vec<String>,
    pub tile_matrix_set_links: Vec<String>,
    pub bounding_box: BoundingBox,
    pub dimensions: Vec<WmtsDimensionInfo>,
}

#[derive(Debug, Clone)]
pub struct WmtsStyleInfo {
    pub identifier: String,
    pub title: String,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct WmtsDimensionInfo {
    pub identifier: String,
    pub default: String,
    pub values: Vec<String>,
}

impl WmtsCapabilitiesBuilder {
    pub fn build(&self) -> String {
        let mut xml = String::new();

        xml.push_str(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Capabilities xmlns="http://www.opengis.net/wmts/1.0"
    xmlns:ows="http://www.opengis.net/ows/1.1"
    xmlns:xlink="http://www.w3.org/1999/xlink"
    xmlns:gml="http://www.opengis.net/gml"
    version="1.0.0">
"#,
        );

        // ServiceIdentification
        xml.push_str(&format!(
            r#"  <ows:ServiceIdentification>
    <ows:Title>{}</ows:Title>
    <ows:Abstract>{}</ows:Abstract>
    <ows:ServiceType>OGC WMTS</ows:ServiceType>
    <ows:ServiceTypeVersion>1.0.0</ows:ServiceTypeVersion>
  </ows:ServiceIdentification>
"#,
            self.service_title, self.service_abstract
        ));

        // ServiceProvider (minimal)
        xml.push_str(
            r#"  <ows:ServiceProvider>
    <ows:ProviderName>Weather WMS</ows:ProviderName>
  </ows:ServiceProvider>
"#,
        );

        // OperationsMetadata
        xml.push_str(&format!(
            r#"  <ows:OperationsMetadata>
    <ows:Operation name="GetCapabilities">
      <ows:DCP>
        <ows:HTTP>
          <ows:Get xlink:href="{0}/wmts?">
            <ows:Constraint name="GetEncoding">
              <ows:AllowedValues><ows:Value>KVP</ows:Value></ows:AllowedValues>
            </ows:Constraint>
          </ows:Get>
        </ows:HTTP>
      </ows:DCP>
    </ows:Operation>
    <ows:Operation name="GetTile">
      <ows:DCP>
        <ows:HTTP>
          <ows:Get xlink:href="{0}/wmts?">
            <ows:Constraint name="GetEncoding">
              <ows:AllowedValues><ows:Value>KVP</ows:Value></ows:AllowedValues>
            </ows:Constraint>
          </ows:Get>
          <ows:Get xlink:href="{0}/wmts/rest/">
            <ows:Constraint name="GetEncoding">
              <ows:AllowedValues><ows:Value>RESTful</ows:Value></ows:AllowedValues>
            </ows:Constraint>
          </ows:Get>
        </ows:HTTP>
      </ows:DCP>
    </ows:Operation>
  </ows:OperationsMetadata>
"#,
            self.service_url
        ));

        // Contents
        xml.push_str("  <Contents>\n");

        // Layers
        for layer in &self.layers {
            xml.push_str(&format!(
                r#"    <Layer>
      <ows:Title>{}</ows:Title>
      <ows:Identifier>{}</ows:Identifier>
"#,
                layer.title, layer.identifier
            ));

            // Bounding box
            xml.push_str(&format!(
                r#"      <ows:WGS84BoundingBox>
        <ows:LowerCorner>{} {}</ows:LowerCorner>
        <ows:UpperCorner>{} {}</ows:UpperCorner>
      </ows:WGS84BoundingBox>
"#,
                layer.bounding_box.min_x,
                layer.bounding_box.min_y,
                layer.bounding_box.max_x,
                layer.bounding_box.max_y
            ));

            // Styles
            for style in &layer.styles {
                let default = if style.is_default {
                    "\n        <ows:Keyword>default</ows:Keyword>"
                } else {
                    ""
                };
                xml.push_str(&format!(
                    r#"      <Style isDefault="{}">
        <ows:Title>{}</ows:Title>
        <ows:Identifier>{}</ows:Identifier>{}
      </Style>
"#,
                    style.is_default, style.title, style.identifier, default
                ));
            }

            // Formats
            for format in &layer.formats {
                xml.push_str(&format!("      <Format>{}</Format>\n", format));
            }

            // TileMatrixSetLinks
            for tms_link in &layer.tile_matrix_set_links {
                xml.push_str(&format!(
                    r#"      <TileMatrixSetLink>
        <TileMatrixSet>{}</TileMatrixSet>
      </TileMatrixSetLink>
"#,
                    tms_link
                ));
            }

            // Dimensions (TIME, etc.)
            for dim in &layer.dimensions {
                xml.push_str(&format!(
                    r#"      <Dimension>
        <ows:Identifier>{}</ows:Identifier>
        <Default>{}</Default>
        <Value>{}</Value>
      </Dimension>
"#,
                    dim.identifier,
                    dim.default,
                    dim.values.join(",")
                ));
            }

            // ResourceURL (RESTful)
            xml.push_str(&format!(
                r#"      <ResourceURL format="image/png" resourceType="tile" template="{}/wmts/rest/{{}}/{{style}}/{{TileMatrixSet}}/{{TileMatrix}}/{{TileRow}}/{{TileCol}}.png"/>
"#, self.service_url));

            xml.push_str("    </Layer>\n");
        }

        // TileMatrixSets
        for tms in &self.tile_matrix_sets {
            xml.push_str(&format!(
                r#"    <TileMatrixSet>
      <ows:Identifier>{}</ows:Identifier>
      <ows:SupportedCRS>urn:ogc:def:crs:EPSG::{}</ows:SupportedCRS>
"#,
                tms.identifier,
                crs_to_epsg_code(&tms.crs)
            ));

            if let Some(ref wkss) = tms.well_known_scale_set {
                xml.push_str(&format!(
                    "      <WellKnownScaleSet>{}</WellKnownScaleSet>\n",
                    wkss
                ));
            }

            for matrix in &tms.tile_matrices {
                xml.push_str(&format!(
                    r#"      <TileMatrix>
        <ows:Identifier>{}</ows:Identifier>
        <ScaleDenominator>{}</ScaleDenominator>
        <TopLeftCorner>{} {}</TopLeftCorner>
        <TileWidth>{}</TileWidth>
        <TileHeight>{}</TileHeight>
        <MatrixWidth>{}</MatrixWidth>
        <MatrixHeight>{}</MatrixHeight>
      </TileMatrix>
"#,
                    matrix.identifier,
                    matrix.scale_denominator,
                    matrix.top_left_corner.0,
                    matrix.top_left_corner.1,
                    matrix.tile_width,
                    matrix.tile_height,
                    matrix.matrix_width,
                    matrix.matrix_height
                ));
            }

            xml.push_str("    </TileMatrixSet>\n");
        }

        xml.push_str("  </Contents>\n");
        xml.push_str("</Capabilities>\n");

        xml
    }
}

fn crs_to_epsg_code(crs: &CrsCode) -> u32 {
    match crs {
        CrsCode::Epsg4326 => 4326,
        CrsCode::Epsg3857 => 3857,
        CrsCode::Epsg4269 => 4269,
        CrsCode::Epsg5070 => 5070,
        CrsCode::Epsg3413 => 3413,
        CrsCode::Epsg3031 => 3031,
    }
}

/// Generate WMTS exception XML.
pub fn wmts_exception(code: &str, message: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ows:ExceptionReport xmlns:ows="http://www.opengis.net/ows/1.1" version="1.0.0">
  <ows:Exception exceptionCode="{}">
    <ows:ExceptionText>{}</ows:ExceptionText>
  </ows:Exception>
</ows:ExceptionReport>"#,
        code, message
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rest_path_parsing() {
        let path = "/gfs_temp/gradient/WebMercatorQuad/5/10/15.png";
        let parsed = WmtsRestPath::parse(path).unwrap();

        assert_eq!(parsed.layer, "gfs_temp");
        assert_eq!(parsed.style, "gradient");
        assert_eq!(parsed.tile_matrix_set, "WebMercatorQuad");
        assert_eq!(parsed.tile_matrix, "5");
        assert_eq!(parsed.tile_row, 10);
        assert_eq!(parsed.tile_col, 15);
        assert_eq!(parsed.format, "image/png");
    }

    #[test]
    fn test_rest_path_with_time() {
        let path = "/gfs_temp/gradient/2024-01-15T12:00:00Z/WebMercatorQuad/5/10/15.png";
        let parsed = WmtsRestPath::parse(path).unwrap();

        assert_eq!(parsed.layer, "gfs_temp");
        assert_eq!(parsed.time, Some("2024-01-15T12:00:00Z".to_string()));
    }

    #[test]
    fn test_cache_key() {
        let req = GetTileRequest {
            layer: "temperature".to_string(),
            style: "gradient".to_string(),
            format: "image/png".to_string(),
            tile_matrix_set: "WebMercatorQuad".to_string(),
            tile_matrix: "5".to_string(),
            tile_row: 10,
            tile_col: 15,
            time: Some("2024-01-15T12:00:00Z".to_string()),
            elevation: None,
            dimensions: std::collections::HashMap::new(),
        };

        let key = req.cache_key();
        assert!(key.contains("temperature"));
        assert!(key.contains("10"));
        assert!(key.contains("15"));
    }
}
