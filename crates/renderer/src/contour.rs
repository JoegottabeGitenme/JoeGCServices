//! Contour line (isoline) rendering using marching squares algorithm.
//!
//! This module implements contour generation for gridded data, producing
//! smooth anti-aliased lines that can be rendered across tile boundaries.

/// A point in 2D space (pixel coordinates)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A line segment between two points
#[derive(Debug, Clone)]
pub struct Segment {
    pub start: Point,
    pub end: Point,
}

/// A complete contour line (polyline)
#[derive(Debug, Clone)]
pub struct Contour {
    pub level: f32,
    pub points: Vec<Point>,
    pub closed: bool,
}

/// Configuration for contour rendering
#[derive(Debug, Clone)]
pub struct ContourConfig {
    /// Contour levels to draw
    pub levels: Vec<f32>,
    /// Line width in pixels
    pub line_width: f32,
    /// Line color [R, G, B, A]
    pub line_color: [u8; 4],
    /// Number of smoothing passes (0 = no smoothing)
    pub smoothing_passes: u32,
}

impl Default for ContourConfig {
    fn default() -> Self {
        Self {
            levels: vec![],
            line_width: 2.0,
            line_color: [0, 0, 0, 255],
            smoothing_passes: 1,
        }
    }
}

/// Generate contour levels automatically based on data range and interval
pub fn generate_contour_levels(min_value: f32, max_value: f32, interval: f32) -> Vec<f32> {
    if interval <= 0.0 || max_value <= min_value {
        return vec![];
    }
    
    // Start from first multiple of interval above min_value
    let start = (min_value / interval).ceil() * interval;
    let mut levels = Vec::new();
    
    let mut level = start;
    while level <= max_value {
        levels.push(level);
        level += interval;
    }
    
    levels
}

/// Marching squares algorithm to generate contour lines
///
/// # Arguments
/// * `data` - Grid data in row-major order
/// * `width` - Grid width
/// * `height` - Grid height
/// * `level` - Contour level to extract
///
/// # Returns
/// Vector of line segments representing the contour
pub fn march_squares(data: &[f32], width: usize, height: usize, level: f32) -> Vec<Segment> {
    if width < 2 || height < 2 || data.len() != width * height {
        return vec![];
    }
    
    let mut segments = Vec::new();
    
    // Iterate over each cell in the grid
    for y in 0..(height - 1) {
        for x in 0..(width - 1) {
            // Get the four corners of the cell
            let tl = data[y * width + x];           // top-left
            let tr = data[y * width + x + 1];       // top-right
            let bl = data[(y + 1) * width + x];     // bottom-left
            let br = data[(y + 1) * width + x + 1]; // bottom-right
            
            // Skip cells with NaN values
            if tl.is_nan() || tr.is_nan() || bl.is_nan() || br.is_nan() {
                continue;
            }
            
            // Calculate cell index (0-15) based on which corners are above the threshold
            let mut cell_index = 0;
            if tl >= level { cell_index |= 1; }
            if tr >= level { cell_index |= 2; }
            if br >= level { cell_index |= 4; }
            if bl >= level { cell_index |= 8; }
            
            // Get segments for this cell based on marching squares lookup
            let cell_segments = get_cell_segments(
                cell_index,
                x as f32, y as f32,
                tl, tr, br, bl,
                level
            );
            
            segments.extend(cell_segments);
        }
    }
    
    segments
}

/// Get line segments for a marching squares cell
///
/// Uses linear interpolation to find where the contour crosses cell edges
fn get_cell_segments(
    cell_index: u8,
    x: f32,
    y: f32,
    tl: f32,
    tr: f32,
    br: f32,
    bl: f32,
    level: f32,
) -> Vec<Segment> {
    // Edge midpoints (will be interpolated)
    // Top edge: between tl and tr
    // Right edge: between tr and br
    // Bottom edge: between bl and br
    // Left edge: between tl and bl
    
    let top = interpolate_edge(x, y, x + 1.0, y, tl, tr, level);
    let right = interpolate_edge(x + 1.0, y, x + 1.0, y + 1.0, tr, br, level);
    let bottom = interpolate_edge(x, y + 1.0, x + 1.0, y + 1.0, bl, br, level);
    let left = interpolate_edge(x, y, x, y + 1.0, tl, bl, level);
    
    // Marching squares lookup table
    // Each case defines which edges to connect
    match cell_index {
        0 | 15 => vec![], // All same side - no contour
        1 | 14 => vec![Segment { start: left, end: top }],
        2 | 13 => vec![Segment { start: top, end: right }],
        3 | 12 => vec![Segment { start: left, end: right }],
        4 | 11 => vec![Segment { start: right, end: bottom }],
        5 => vec![ // Saddle case - two separate segments
            Segment { start: left, end: top },
            Segment { start: right, end: bottom },
        ],
        6 | 9 => vec![Segment { start: top, end: bottom }],
        7 | 8 => vec![Segment { start: left, end: bottom }],
        10 => vec![ // Saddle case - two separate segments
            Segment { start: top, end: right },
            Segment { start: left, end: bottom },
        ],
        _ => vec![],
    }
}

/// Linearly interpolate between two edge points based on data values
fn interpolate_edge(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    val1: f32,
    val2: f32,
    level: f32,
) -> Point {
    if (val2 - val1).abs() < 1e-6 {
        // Values are essentially equal, use midpoint
        return Point::new((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    }
    
    // Linear interpolation: find where level crosses between val1 and val2
    let t = (level - val1) / (val2 - val1);
    let t = t.clamp(0.0, 1.0);
    
    Point::new(
        x1 + t * (x2 - x1),
        y1 + t * (y2 - y1),
    )
}

/// Connect line segments into continuous polylines
///
/// Takes a collection of unordered segments and tries to connect them
/// into continuous contour lines.
pub fn connect_segments(segments: Vec<Segment>) -> Vec<Contour> {
    if segments.is_empty() {
        return vec![];
    }
    
    let mut contours = Vec::new();
    let mut used = vec![false; segments.len()];
    let epsilon = 0.001; // Tolerance for point matching
    
    for start_idx in 0..segments.len() {
        if used[start_idx] {
            continue;
        }
        
        let mut points = vec![segments[start_idx].start, segments[start_idx].end];
        used[start_idx] = true;
        
        let mut changed = true;
        while changed {
            changed = false;
            let current_end = *points.last().unwrap();
            
            // Try to find a segment that starts where we ended
            for i in 0..segments.len() {
                if used[i] {
                    continue;
                }
                
                let seg = &segments[i];
                let dist_start = ((seg.start.x - current_end.x).powi(2) + 
                                 (seg.start.y - current_end.y).powi(2)).sqrt();
                let dist_end = ((seg.end.x - current_end.x).powi(2) + 
                               (seg.end.y - current_end.y).powi(2)).sqrt();
                
                if dist_start < epsilon {
                    points.push(seg.end);
                    used[i] = true;
                    changed = true;
                    break;
                } else if dist_end < epsilon {
                    points.push(seg.start);
                    used[i] = true;
                    changed = true;
                    break;
                }
            }
        }
        
        // Check if contour is closed
        let first = points[0];
        let last = *points.last().unwrap();
        let closed = ((first.x - last.x).powi(2) + (first.y - last.y).powi(2)).sqrt() < epsilon;
        
        if points.len() >= 2 {
            contours.push(Contour {
                level: 0.0, // Level will be set by caller
                points,
                closed,
            });
        }
    }
    
    contours
}

/// Apply Chaikin's corner cutting algorithm for smoothing
pub fn smooth_contour(contour: &Contour, iterations: u32) -> Contour {
    if iterations == 0 || contour.points.len() < 3 {
        return contour.clone();
    }
    
    let mut points = contour.points.clone();
    
    for _ in 0..iterations {
        let mut new_points = Vec::with_capacity(points.len() * 2);
        
        for i in 0..points.len() {
            let p1 = points[i];
            let p2 = if contour.closed {
                points[(i + 1) % points.len()]
            } else if i + 1 < points.len() {
                points[i + 1]
            } else {
                break;
            };
            
            // Create two new points: 25% and 75% along the segment
            let q = Point::new(
                0.75 * p1.x + 0.25 * p2.x,
                0.75 * p1.y + 0.25 * p2.y,
            );
            let r = Point::new(
                0.25 * p1.x + 0.75 * p2.x,
                0.25 * p1.y + 0.75 * p2.y,
            );
            
            new_points.push(q);
            new_points.push(r);
        }
        
        // If not closed, keep the endpoints
        if !contour.closed && !points.is_empty() {
            new_points.insert(0, points[0]);
            if let Some(&last) = points.last() {
                new_points.push(last);
            }
        }
        
        points = new_points;
    }
    
    Contour {
        level: contour.level,
        points,
        closed: contour.closed,
    }
}

/// Render contours to an RGBA canvas using tiny-skia
pub fn render_contours_to_canvas(
    contours: &[Contour],
    width: usize,
    height: usize,
    config: &ContourConfig,
) -> Vec<u8> {
    use tiny_skia::*;
    
    // Create pixmap
    let mut pixmap = Pixmap::new(width as u32, height as u32)
        .expect("Failed to create pixmap");
    
    // Fill with transparent
    pixmap.fill(Color::TRANSPARENT);
    
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        config.line_color[0],
        config.line_color[1],
        config.line_color[2],
        config.line_color[3],
    );
    paint.anti_alias = true;
    
    let mut stroke = Stroke::default();
    stroke.width = config.line_width;
    stroke.line_cap = LineCap::Round;
    stroke.line_join = LineJoin::Round;
    
    // Draw each contour
    for contour in contours {
        if contour.points.len() < 2 {
            continue;
        }
        
        let mut pb = PathBuilder::new();
        
        // Start path
        pb.move_to(contour.points[0].x, contour.points[0].y);
        
        // Add remaining points
        for point in &contour.points[1..] {
            pb.line_to(point.x, point.y);
        }
        
        // Close if needed
        if contour.closed {
            pb.close();
        }
        
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
    
    // Convert to RGBA bytes
    pixmap.data().to_vec()
}

/// Generate all contours for multiple levels
pub fn generate_all_contours(
    data: &[f32],
    width: usize,
    height: usize,
    config: &ContourConfig,
) -> Vec<Contour> {
    let mut all_contours = Vec::new();
    
    for &level in &config.levels {
        // Generate segments for this level
        let segments = march_squares(data, width, height, level);
        
        // Connect segments into contours
        let mut contours = connect_segments(segments);
        
        // Set level and smooth
        for contour in &mut contours {
            contour.level = level;
            if config.smoothing_passes > 0 {
                *contour = smooth_contour(contour, config.smoothing_passes);
            }
        }
        
        all_contours.extend(contours);
    }
    
    all_contours
}

/// High-level function to render contours from data
///
/// This is the main entry point for contour rendering from the service layer
pub fn render_contours(
    data: &[f32],
    width: usize,
    height: usize,
    config: &ContourConfig,
) -> Vec<u8> {
    // Generate all contours
    let contours = generate_all_contours(data, width, height, config);
    
    // Render to canvas
    render_contours_to_canvas(&contours, width, height, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_generate_contour_levels() {
        let levels = generate_contour_levels(0.0, 20.0, 5.0);
        assert_eq!(levels, vec![0.0, 5.0, 10.0, 15.0, 20.0]);
        
        let levels = generate_contour_levels(2.0, 18.0, 5.0);
        assert_eq!(levels, vec![5.0, 10.0, 15.0]);
    }
    
    #[test]
    fn test_interpolate_edge() {
        let p = interpolate_edge(0.0, 0.0, 1.0, 0.0, 0.0, 10.0, 5.0);
        assert!((p.x - 0.5).abs() < 0.01);
        assert!((p.y - 0.0).abs() < 0.01);
    }
    
    #[test]
    fn test_march_squares_flat() {
        let data = vec![5.0; 9];
        let segments = march_squares(&data, 3, 3, 5.0);
        assert_eq!(segments.len(), 0); // No contour for flat field
    }
    
    #[test]
    fn test_march_squares_simple() {
        // Simple 3x3 grid with peak in center
        let data = vec![
            0.0, 0.0, 0.0,
            0.0, 10.0, 0.0,
            0.0, 0.0, 0.0,
        ];
        let segments = march_squares(&data, 3, 3, 5.0);
        assert!(segments.len() > 0); // Should generate contour around peak
    }
}
