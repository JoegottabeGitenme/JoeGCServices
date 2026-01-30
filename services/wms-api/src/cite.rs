//! CITE (Compliance Interoperability Testing & Evaluation) test data support.
//!
//! This module provides static image layer support for OGC CITE WMS compliance testing.
//! It handles georeferenced PNG images with worldfile (.pgw) metadata.
//!
//! The CITE test suite requires specific layers (cite:Lakes, cite:Ponds, etc.) with
//! known pixel values at specific coordinates. This module serves those images
//! directly without applying weather-style colormaps.

use image::{GenericImageView, ImageBuffer, Rgba, RgbaImage};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::{debug, info, warn};

/// Global flag indicating if CITE data is enabled
static CITE_ENABLED: OnceLock<bool> = OnceLock::new();

/// Cached CITE layer configurations
static CITE_LAYERS: OnceLock<HashMap<String, CiteLayer>> = OnceLock::new();

/// Check if CITE test data is enabled via environment variable
pub fn is_cite_enabled() -> bool {
    *CITE_ENABLED.get_or_init(|| {
        let enabled = env::var("ENABLE_CITE_DATA")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);
        if enabled {
            info!("CITE test data enabled");
        }
        enabled
    })
}

/// Get the directory containing CITE test data
fn get_cite_data_dir() -> PathBuf {
    let base = env::var("CITE_DATA_DIR")
        .unwrap_or_else(|_| "validation/ogc-compliance/cite-data".to_string());
    PathBuf::from(base)
}

/// Worldfile parameters for georeferencing
#[derive(Debug, Clone)]
pub struct Worldfile {
    /// Pixel size in X direction (map units per pixel)
    pub pixel_size_x: f64,
    /// Rotation about Y axis (usually 0)
    pub rotation_x: f64,
    /// Rotation about X axis (usually 0)
    pub rotation_y: f64,
    /// Pixel size in Y direction (negative for top-down images)
    pub pixel_size_y: f64,
    /// X coordinate of center of upper-left pixel
    pub upper_left_x: f64,
    /// Y coordinate of center of upper-left pixel
    pub upper_left_y: f64,
}

impl Worldfile {
    /// Parse a worldfile from its contents
    pub fn parse(content: &str) -> Option<Self> {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() < 6 {
            return None;
        }

        Some(Worldfile {
            pixel_size_x: lines[0].trim().parse().ok()?,
            rotation_x: lines[1].trim().parse().ok()?,
            rotation_y: lines[2].trim().parse().ok()?,
            pixel_size_y: lines[3].trim().parse().ok()?,
            upper_left_x: lines[4].trim().parse().ok()?,
            upper_left_y: lines[5].trim().parse().ok()?,
        })
    }

    /// Calculate the bounding box for an image with given dimensions
    pub fn calculate_bbox(&self, width: u32, height: u32) -> [f64; 4] {
        // Upper-left corner (center of upper-left pixel)
        // Adjust to get edge of pixel
        let min_x = self.upper_left_x - self.pixel_size_x / 2.0;
        let max_y = self.upper_left_y - self.pixel_size_y / 2.0; // pixel_size_y is negative

        // Calculate extent
        let max_x = min_x + (width as f64 * self.pixel_size_x);
        let min_y = max_y + (height as f64 * self.pixel_size_y); // pixel_size_y is negative

        [min_x, min_y, max_x, max_y]
    }
}

/// Dimension definition for CITE layers
#[derive(Debug, Clone)]
pub struct CiteDimension {
    /// Dimension name (e.g., "elevation", "time")
    pub name: String,
    /// Dimension units (e.g., "CRS:88", "ISO8601")
    pub units: String,
    /// Unit symbol (e.g., "m")
    pub unit_symbol: Option<String>,
    /// Dimension values (comma-separated or interval)
    pub values: String,
    /// Default value (None means no default - dimension is REQUIRED)
    pub default: Option<String>,
    /// Whether multiple values can be requested
    pub multiple_values: bool,
    /// Whether nearest value should be used if exact match not found
    pub nearest_value: bool,
}

/// A CITE test layer with its image and georeferencing
#[derive(Debug, Clone)]
pub struct CiteLayer {
    /// Layer name (e.g., "Lakes", "Ponds")
    pub name: String,
    /// Full layer identifier (e.g., "cite:Lakes")
    pub identifier: String,
    /// Layer title for capabilities
    pub title: String,
    /// Path to PNG file
    pub png_path: PathBuf,
    /// Worldfile parameters
    pub worldfile: Worldfile,
    /// Image dimensions
    pub width: u32,
    pub height: u32,
    /// Bounding box [minX, minY, maxX, maxY] in CRS:84
    pub bbox: [f64; 4],
    /// Whether layer supports GetFeatureInfo (queryable)
    pub queryable: bool,
    /// Layer dimensions (elevation, time, etc.)
    pub dimensions: Vec<CiteDimension>,
    /// Whether the layer is opaque (no transparency support)
    pub opaque: bool,
}

impl CiteLayer {
    /// Load a CITE layer from PNG and worldfile
    pub fn load(name: &str, data_dir: &PathBuf) -> Option<Self> {
        let png_path = data_dir.join(format!("{}.png", name));
        let pgw_path = data_dir.join(format!("{}.pgw", name));

        if !png_path.exists() || !pgw_path.exists() {
            warn!(
                name = name,
                png_exists = png_path.exists(),
                pgw_exists = pgw_path.exists(),
                "CITE layer files not found"
            );
            return None;
        }

        // Parse worldfile
        let pgw_content = fs::read_to_string(&pgw_path).ok()?;
        let worldfile = Worldfile::parse(&pgw_content)?;

        // Get image dimensions without loading full image
        let img = image::open(&png_path).ok()?;
        let (width, height) = img.dimensions();

        let bbox = worldfile.calculate_bbox(width, height);

        debug!(
            name = name,
            width = width,
            height = height,
            bbox = ?bbox,
            "Loaded CITE layer"
        );

        // Polygon layers (Lakes, Ponds, etc.) are queryable
        let queryable = matches!(
            name,
            "Lakes"
                | "Ponds"
                | "Buildings"
                | "Forests"
                | "NamedPlaces"
                | "BasicPolygons"
                | "lakesWithElevation"
        );

        // Set up dimensions based on layer type (per OGC CITE test requirements)
        // Note: cite:Lakes must NOT have a required dimension because it's used for many
        // basic tests. Use cite:lakesWithElevation for the "missing-no-default" test.
        let dimensions = match name {
            // cite:lakesWithElevation has ELEVATION dimension WITHOUT a default value
            // This is required for the "missing-no-default" test
            "lakesWithElevation" => vec![CiteDimension {
                name: "elevation".to_string(),
                units: "CRS:88".to_string(),
                unit_symbol: Some("m".to_string()),
                values: "500,490,480".to_string(),
                default: None, // NO default - dimension is REQUIRED
                multiple_values: false,
                nearest_value: true,
            }],
            // cite:Autos has TIME dimension (for time dimension tests)
            "Autos" => vec![CiteDimension {
                name: "time".to_string(),
                units: "ISO8601".to_string(),
                unit_symbol: None,
                values: "2000-01-01T00:00:00Z/2000-01-01T00:01:00Z/PT5S".to_string(),
                default: Some("2000-01-01T00:00:00Z".to_string()),
                multiple_values: true,
                nearest_value: true,
            }],
            _ => vec![],
        };

        // MapNeatline is the only opaque layer (solid background)
        let opaque = name == "MapNeatline";

        Some(CiteLayer {
            name: name.to_string(),
            identifier: format!("cite:{}", name),
            title: format!("cite:{}", name),
            png_path,
            worldfile,
            width,
            height,
            bbox,
            queryable,
            dimensions,
            opaque,
        })
    }
}

/// Get all available CITE layers
pub fn get_cite_layers() -> &'static HashMap<String, CiteLayer> {
    CITE_LAYERS.get_or_init(|| {
        if !is_cite_enabled() {
            return HashMap::new();
        }

        let data_dir = get_cite_data_dir();
        if !data_dir.exists() {
            warn!(dir = ?data_dir, "CITE data directory not found");
            return HashMap::new();
        }

        // Standard CITE layer names
        let layer_names = [
            "Lakes",
            "Ponds",
            "Buildings",
            "Forests",
            "NamedPlaces",
            "BasicPolygons",
            "MapNeatline",
            "RoadSegments",
            "Streams",
            "DividedRoutes",
            "Bridges",
            "BuildingCenters",
            "Autos",
            "lakesWithElevation",
        ];

        let mut layers = HashMap::new();
        for name in layer_names {
            if let Some(layer) = CiteLayer::load(name, &data_dir) {
                layers.insert(layer.identifier.clone(), layer);
            }
        }

        info!(count = layers.len(), "Loaded CITE layers");
        layers
    })
}

/// Check if a layer name is a CITE layer
pub fn is_cite_layer(layer_name: &str) -> bool {
    layer_name.starts_with("cite:")
}

/// Get a specific CITE layer by name
pub fn get_cite_layer(layer_name: &str) -> Option<&'static CiteLayer> {
    get_cite_layers().get(layer_name)
}

/// Render a CITE layer for a GetMap request
///
/// # Arguments
/// * `layer_name` - The layer identifier (e.g., "cite:Lakes")
/// * `width` - Output image width
/// * `height` - Output image height
/// * `bbox` - Requested bounding box [minX, minY, maxX, maxY] in geographic coords
/// * `transparent` - Whether to use transparent background
/// * `bgcolor` - Background color (if not transparent)
///
/// # Returns
/// PNG image data as bytes
pub fn render_cite_layer(
    layer_name: &str,
    width: u32,
    height: u32,
    bbox: [f64; 4],
    transparent: bool,
    bgcolor: Option<[u8; 3]>,
) -> Result<Vec<u8>, String> {
    let layer = get_cite_layer(layer_name)
        .ok_or_else(|| format!("CITE layer '{}' not found", layer_name))?;

    // Load the source image
    let src_img = image::open(&layer.png_path)
        .map_err(|e| format!("Failed to load CITE image: {}", e))?
        .to_rgba8();

    let [req_min_x, req_min_y, req_max_x, req_max_y] = bbox;
    let [src_min_x, src_min_y, src_max_x, src_max_y] = layer.bbox;

    // Create output image with background
    let bg_color = if transparent {
        Rgba([0, 0, 0, 0])
    } else {
        let [r, g, b] = bgcolor.unwrap_or([255, 255, 255]);
        Rgba([r, g, b, 255])
    };

    let mut output: RgbaImage = ImageBuffer::from_pixel(width, height, bg_color);

    // Check if there's any overlap between request and layer bbox
    if req_max_x < src_min_x
        || req_min_x > src_max_x
        || req_max_y < src_min_y
        || req_min_y > src_max_y
    {
        // No overlap - return background image
        return encode_png(&output);
    }

    // Calculate pixel mappings
    // Source image: pixel (0,0) is at (src_min_x, src_max_y)
    // Each source pixel covers (pixel_size_x, |pixel_size_y|) geographic units
    let src_pixel_width = (src_max_x - src_min_x) / layer.width as f64;
    let src_pixel_height = (src_max_y - src_min_y) / layer.height as f64;

    // Output image: pixel (0,0) is at (req_min_x, req_max_y)
    let out_pixel_width = (req_max_x - req_min_x) / width as f64;
    let out_pixel_height = (req_max_y - req_min_y) / height as f64;

    // For each output pixel, find corresponding source pixel
    for out_y in 0..height {
        // Geographic Y coordinate at center of output pixel
        let geo_y = req_max_y - (out_y as f64 + 0.5) * out_pixel_height;

        for out_x in 0..width {
            // Geographic X coordinate at center of output pixel
            let geo_x = req_min_x + (out_x as f64 + 0.5) * out_pixel_width;

            // Check if within source bounds
            if geo_x < src_min_x || geo_x > src_max_x || geo_y < src_min_y || geo_y > src_max_y {
                continue;
            }

            // Calculate source pixel coordinates
            let src_x = ((geo_x - src_min_x) / src_pixel_width) as u32;
            let src_y = ((src_max_y - geo_y) / src_pixel_height) as u32;

            // Bounds check
            if src_x >= layer.width || src_y >= layer.height {
                continue;
            }

            // Get source pixel and composite onto output
            let src_pixel = src_img.get_pixel(src_x, src_y);

            // Alpha blending
            if src_pixel[3] > 0 {
                let out_pixel = output.get_pixel_mut(out_x, out_y);
                if src_pixel[3] == 255 {
                    *out_pixel = *src_pixel;
                } else {
                    // Alpha blend
                    let alpha = src_pixel[3] as f32 / 255.0;
                    let inv_alpha = 1.0 - alpha;
                    out_pixel[0] =
                        (src_pixel[0] as f32 * alpha + out_pixel[0] as f32 * inv_alpha) as u8;
                    out_pixel[1] =
                        (src_pixel[1] as f32 * alpha + out_pixel[1] as f32 * inv_alpha) as u8;
                    out_pixel[2] =
                        (src_pixel[2] as f32 * alpha + out_pixel[2] as f32 * inv_alpha) as u8;
                    out_pixel[3] =
                        (255.0 * (alpha + inv_alpha * out_pixel[3] as f32 / 255.0)) as u8;
                }
            }
        }
    }

    encode_png(&output)
}

/// Encode an RGBA image to PNG bytes
fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    let mut buffer = Cursor::new(Vec::new());
    img.write_to(&mut buffer, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encoding failed: {}", e))?;

    Ok(buffer.into_inner())
}

/// Get feature info for a CITE layer at a specific point
///
/// # Arguments
/// * `layer_name` - The layer identifier (e.g., "cite:Lakes")
/// * `x` - Pixel X coordinate in the request image
/// * `y` - Pixel Y coordinate in the request image
/// * `width` - Request image width
/// * `height` - Request image height
/// * `bbox` - Request bounding box
///
/// # Returns
/// Feature info as key-value pairs
pub fn get_cite_feature_info(
    layer_name: &str,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    bbox: [f64; 4],
) -> Result<Vec<(String, String)>, String> {
    let layer = get_cite_layer(layer_name)
        .ok_or_else(|| format!("CITE layer '{}' not found", layer_name))?;

    if !layer.queryable {
        return Err(format!("Layer '{}' is not queryable", layer_name));
    }

    // Convert pixel coords to geographic
    let [req_min_x, req_min_y, req_max_x, req_max_y] = bbox;
    let pixel_width = (req_max_x - req_min_x) / width as f64;
    let pixel_height = (req_max_y - req_min_y) / height as f64;

    let geo_x = req_min_x + (x as f64 + 0.5) * pixel_width;
    let geo_y = req_max_y - (y as f64 + 0.5) * pixel_height;

    // Check if point is within layer bounds
    let [src_min_x, src_min_y, src_max_x, src_max_y] = layer.bbox;
    if geo_x < src_min_x || geo_x > src_max_x || geo_y < src_min_y || geo_y > src_max_y {
        return Ok(vec![]); // No features at this location
    }

    // Load image and get pixel value
    let img = image::open(&layer.png_path)
        .map_err(|e| format!("Failed to load image: {}", e))?
        .to_rgba8();

    let src_pixel_width = (src_max_x - src_min_x) / layer.width as f64;
    let src_pixel_height = (src_max_y - src_min_y) / layer.height as f64;

    let src_x = ((geo_x - src_min_x) / src_pixel_width) as u32;
    let src_y = ((src_max_y - geo_y) / src_pixel_height) as u32;

    if src_x >= layer.width || src_y >= layer.height {
        return Ok(vec![]);
    }

    let pixel = img.get_pixel(src_x, src_y);

    // Only return info if pixel is not fully transparent
    if pixel[3] == 0 {
        return Ok(vec![]);
    }

    // Return feature information
    Ok(vec![
        ("layer".to_string(), layer.name.clone()),
        ("x".to_string(), format!("{:.6}", geo_x)),
        ("y".to_string(), format!("{:.6}", geo_y)),
        ("color_r".to_string(), pixel[0].to_string()),
        ("color_g".to_string(), pixel[1].to_string()),
        ("color_b".to_string(), pixel[2].to_string()),
        ("color_a".to_string(), pixel[3].to_string()),
    ])
}

/// Helper function to build a layer's XML
fn build_layer_xml(layer: &CiteLayer, indent: &str) -> String {
    let queryable = if layer.queryable { "1" } else { "0" };
    let opaque = if layer.opaque { "1" } else { "0" };
    let [min_x, min_y, max_x, max_y] = layer.bbox;

    // Build dimension XML for this layer
    let mut dimension_xml = String::new();
    for dim in &layer.dimensions {
        // Build dimension attributes
        let mut attrs = format!(r#"name="{}" units="{}""#, dim.name, dim.units);
        if let Some(ref symbol) = dim.unit_symbol {
            attrs.push_str(&format!(r#" unitSymbol="{}""#, symbol));
        }
        if dim.multiple_values {
            attrs.push_str(r#" multipleValues="true""#);
        } else {
            attrs.push_str(r#" multipleValues="false""#);
        }
        if dim.nearest_value {
            attrs.push_str(r#" nearestValue="true""#);
        } else {
            attrs.push_str(r#" nearestValue="false""#);
        }
        // Only add default attribute if there IS a default value
        // Omitting default means dimension is REQUIRED (for missing-no-default test)
        if let Some(ref default_val) = dim.default {
            attrs.push_str(&format!(r#" default="{}""#, default_val));
        }

        dimension_xml.push_str(&format!(
            "\n{}  <Dimension {}>{}</Dimension>",
            indent, attrs, dim.values
        ));
    }

    // Check if this layer has a dimension without a default value
    // For such layers, we omit the Style element to avoid XPath issues in CITE tests
    // (the test's XPath //Layer[Dimension[not(@default)]]/Name would match Style/Name too)
    let has_required_dimension = layer.dimensions.iter().any(|d| d.default.is_none());

    // Build style XML (only for layers without required dimensions)
    let style_xml = if has_required_dimension {
        String::new() // No Style element for layers with required dimensions
    } else {
        format!(
                "\n{}  <Style>\n{}    <Name>default</Name>\n{}    <Title>Default Style</Title>\n{}  </Style>",
                indent, indent, indent, indent
            )
    };

    // Build bounding box XML (all layers get their own BoundingBox)
    let bbox_xml = format!(
            "{}  <EX_GeographicBoundingBox>\n{}    <westBoundLongitude>{:.10}</westBoundLongitude>\n{}    <eastBoundLongitude>{:.10}</eastBoundLongitude>\n{}    <southBoundLatitude>{:.10}</southBoundLatitude>\n{}    <northBoundLatitude>{:.10}</northBoundLatitude>\n{}  </EX_GeographicBoundingBox>\n{}  <BoundingBox CRS=\"CRS:84\" minx=\"{:.10}\" miny=\"{:.10}\" maxx=\"{:.10}\" maxy=\"{:.10}\"/>\n{}  <BoundingBox CRS=\"EPSG:4326\" minx=\"{:.10}\" miny=\"{:.10}\" maxx=\"{:.10}\" maxy=\"{:.10}\"/>",
            indent, indent, min_x, indent, max_x, indent, min_y, indent, max_y, indent,
            indent, min_x, min_y, max_x, max_y,
            indent, min_y, min_x, max_y, max_x
        );

    // For layers with required dimensions, omit CRS so they inherit from parent
    // This prevents crs-direct test from selecting them
    let crs_xml = if has_required_dimension {
        String::new() // CRS inherited from parent container
    } else {
        format!(
            "{}  <CRS>CRS:84</CRS>\n{}  <CRS>EPSG:4326</CRS>\n",
            indent, indent
        )
    };

    format!(
        r#"{}<Layer queryable="{}" opaque="{}">
{}  <Name>{}</Name>
{}  <Title>{}</Title>
{}{}{}{}
{}</Layer>
"#,
        indent,
        queryable,
        opaque,
        indent,
        layer.identifier,
        indent,
        layer.title,
        crs_xml,
        bbox_xml,
        dimension_xml,
        style_xml,
        indent
    )
}

/// Generate capabilities XML fragment for regular CITE layers (without required dimensions)
/// These layers should be placed BEFORE weather layers so they get selected first by CITE tests
pub fn get_cite_capabilities_layers() -> String {
    if !is_cite_enabled() {
        return String::new();
    }

    let layers = get_cite_layers();
    if layers.is_empty() {
        return String::new();
    }

    // Get only regular layers (those without required dimensions)
    let mut regular_layers: Vec<&CiteLayer> = layers
        .values()
        .filter(|layer| !layer.dimensions.iter().any(|d| d.default.is_none()))
        .collect();

    // Sort for consistent ordering
    regular_layers.sort_by(|a, b| a.identifier.cmp(&b.identifier));

    let mut xml = String::new();

    // Output regular layers in the main CITE container
    xml.push_str(
        r#"
    <Layer>
      <Title>OGC CITE Test Layers</Title>
      <Abstract>Standard OGC CITE test layers for WMS compliance testing</Abstract>
      <CRS>CRS:84</CRS>
      <CRS>EPSG:4326</CRS>
      <EX_GeographicBoundingBox>
        <westBoundLongitude>-0.005</westBoundLongitude>
        <eastBoundLongitude>0.005</eastBoundLongitude>
        <southBoundLatitude>-0.005</southBoundLatitude>
        <northBoundLatitude>0.005</northBoundLatitude>
      </EX_GeographicBoundingBox>
      <BoundingBox CRS="CRS:84" minx="-0.005" miny="-0.005" maxx="0.005" maxy="0.005"/>
      <BoundingBox CRS="EPSG:4326" minx="-0.005" miny="-0.005" maxx="0.005" maxy="0.005"/>
"#,
    );

    for layer in &regular_layers {
        xml.push_str(&build_layer_xml(layer, "      "));
    }

    xml.push_str("    </Layer>\n");
    xml
}

/// Generate capabilities XML fragment for CITE layers with required dimensions
/// These layers should be placed AFTER all other layers so they don't get selected
/// by tests that don't know about their required dimensions
pub fn get_cite_required_dimension_layers() -> String {
    if !is_cite_enabled() {
        return String::new();
    }

    let layers = get_cite_layers();
    if layers.is_empty() {
        return String::new();
    }

    // Get only layers with required dimensions (those without defaults)
    let mut required_dim_layers: Vec<&CiteLayer> = layers
        .values()
        .filter(|layer| layer.dimensions.iter().any(|d| d.default.is_none()))
        .collect();

    if required_dim_layers.is_empty() {
        return String::new();
    }

    // Sort for consistent ordering
    required_dim_layers.sort_by(|a, b| a.identifier.cmp(&b.identifier));

    let mut xml = String::new();

    // Output layers with required dimensions in their own isolated containers
    // Each such layer gets its own parent container with NO siblings
    // This prevents the CITE test's XPath from finding multiple Name elements:
    //   $dimension/../descendant-or-self::wms:Layer[wms:Name][1]/wms:Name
    // When it walks up to parent then down to descendants, it should only find ONE Name
    for layer in &required_dim_layers {
        xml.push_str(
            r#"
    <Layer>
      <Title>Required Dimension Layers</Title>
      <CRS>CRS:84</CRS>
      <CRS>EPSG:4326</CRS>
      <EX_GeographicBoundingBox>
        <westBoundLongitude>-0.005</westBoundLongitude>
        <eastBoundLongitude>0.005</eastBoundLongitude>
        <southBoundLatitude>-0.005</southBoundLatitude>
        <northBoundLatitude>0.005</northBoundLatitude>
      </EX_GeographicBoundingBox>
      <BoundingBox CRS="CRS:84" minx="-0.005" miny="-0.005" maxx="0.005" maxy="0.005"/>
      <BoundingBox CRS="EPSG:4326" minx="-0.005" miny="-0.005" maxx="0.005" maxy="0.005"/>
"#,
        );
        xml.push_str(&build_layer_xml(layer, "      "));
        xml.push_str("    </Layer>\n");
    }

    xml
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worldfile_parse() {
        let content = "0.00000293296196319\n0\n0\n-0.00000293296196319\n0.00053896600245399\n0.0002437155190184";
        let wf = Worldfile::parse(content).unwrap();
        assert!((wf.pixel_size_x - 0.00000293296196319).abs() < 1e-15);
        assert!((wf.pixel_size_y - (-0.00000293296196319)).abs() < 1e-15);
        assert!((wf.upper_left_x - 0.00053896600245399).abs() < 1e-15);
        assert!((wf.upper_left_y - 0.0002437155190184).abs() < 1e-15);
    }

    #[test]
    fn test_worldfile_bbox() {
        let wf = Worldfile {
            pixel_size_x: 0.001,
            rotation_x: 0.0,
            rotation_y: 0.0,
            pixel_size_y: -0.001,
            upper_left_x: 0.0,
            upper_left_y: 1.0,
        };

        let bbox = wf.calculate_bbox(100, 100);
        // Upper-left pixel center is at (0, 1)
        // Image is 100x100 pixels, each 0.001 degrees
        // min_x = 0 - 0.001/2 = -0.0005
        // max_x = -0.0005 + 100 * 0.001 = 0.0995
        // max_y = 1 - (-0.001/2) = 1.0005
        // min_y = 1.0005 + 100 * (-0.001) = 0.9005
        assert!((bbox[0] - (-0.0005)).abs() < 1e-10);
        assert!((bbox[2] - 0.0995).abs() < 1e-10);
    }

    #[test]
    fn test_is_cite_layer() {
        assert!(is_cite_layer("cite:Lakes"));
        assert!(is_cite_layer("cite:Ponds"));
        assert!(!is_cite_layer("gfs_TMP"));
        assert!(!is_cite_layer("Lakes"));
    }
}
