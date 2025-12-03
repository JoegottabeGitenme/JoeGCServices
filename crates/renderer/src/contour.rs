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
    /// Whether to draw labels on contour lines
    pub labels_enabled: bool,
    /// Font size for labels
    pub label_font_size: f32,
    /// Minimum spacing between labels (in pixels)
    pub label_spacing: f32,
    /// Unit conversion offset for label display (e.g., -273.15 to show Celsius)
    pub label_unit_offset: f32,
    /// Special level styling overrides
    pub special_levels: Vec<SpecialLevelConfig>,
}

/// Special styling for a specific contour level
#[derive(Debug, Clone)]
pub struct SpecialLevelConfig {
    /// The level value (in data units)
    pub level: f32,
    /// Custom line color for this level
    pub line_color: Option<[u8; 4]>,
    /// Custom line width for this level
    pub line_width: Option<f32>,
    /// Custom label text (overrides numeric value)
    pub label: Option<String>,
}

impl Default for ContourConfig {
    fn default() -> Self {
        Self {
            levels: vec![],
            line_width: 2.0,
            line_color: [0, 0, 0, 255],
            smoothing_passes: 1,
            labels_enabled: false,
            label_font_size: 10.0,
            label_spacing: 150.0,
            label_unit_offset: 0.0,
            special_levels: vec![],
        }
    }
}

impl ContourConfig {
    /// Get the color for a specific level, checking special levels first
    pub fn get_level_color(&self, level: f32) -> [u8; 4] {
        for special in &self.special_levels {
            if (special.level - level).abs() < 0.01 {
                if let Some(color) = special.line_color {
                    return color;
                }
            }
        }
        self.line_color
    }
    
    /// Get the line width for a specific level, checking special levels first
    pub fn get_level_width(&self, level: f32) -> f32 {
        for special in &self.special_levels {
            if (special.level - level).abs() < 0.01 {
                if let Some(width) = special.line_width {
                    return width;
                }
            }
        }
        self.line_width
    }
    
    /// Get the label text for a level
    pub fn get_level_label(&self, level: f32) -> String {
        for special in &self.special_levels {
            if (special.level - level).abs() < 0.01 {
                if let Some(ref label) = special.label {
                    return label.clone();
                }
            }
        }
        // Default: show numeric value with unit offset applied
        let display_value = level + self.label_unit_offset;
        if display_value.fract().abs() < 0.01 {
            format!("{:.0}", display_value)
        } else {
            format!("{:.1}", display_value)
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
    
    // Collect label positions to avoid overlaps
    let mut label_positions: Vec<LabelPosition> = Vec::new();
    
    // Draw each contour with per-level styling
    for contour in contours {
        if contour.points.len() < 2 {
            continue;
        }
        
        // Get level-specific styling
        let line_color = config.get_level_color(contour.level);
        let line_width = config.get_level_width(contour.level);
        
        let mut paint = Paint::default();
        paint.set_color_rgba8(line_color[0], line_color[1], line_color[2], line_color[3]);
        paint.anti_alias = true;
        
        let mut stroke = Stroke::default();
        stroke.width = line_width;
        stroke.line_cap = LineCap::Round;
        stroke.line_join = LineJoin::Round;
        
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
        
        // Collect label positions along this contour
        if config.labels_enabled {
            collect_label_positions(contour, config, &mut label_positions, width, height);
        }
    }
    
    // Draw labels after all contours are drawn
    if config.labels_enabled && !label_positions.is_empty() {
        draw_labels(&mut pixmap, &label_positions, config);
    }
    
    // Convert to RGBA bytes
    pixmap.data().to_vec()
}

/// Position and metadata for a contour label
#[derive(Debug, Clone)]
struct LabelPosition {
    x: f32,
    y: f32,
    angle: f32,  // Rotation angle in radians
    text: String,
    level: f32,
}

/// Calculate the total length of a contour
fn contour_length(contour: &Contour) -> f32 {
    let mut length = 0.0;
    for i in 1..contour.points.len() {
        let dx = contour.points[i].x - contour.points[i-1].x;
        let dy = contour.points[i].y - contour.points[i-1].y;
        length += (dx * dx + dy * dy).sqrt();
    }
    length
}

/// Collect label positions along a contour line
fn collect_label_positions(
    contour: &Contour,
    config: &ContourConfig,
    positions: &mut Vec<LabelPosition>,
    width: usize,
    height: usize,
) {
    let total_length = contour_length(contour);
    if total_length < config.label_spacing * 0.5 {
        return; // Contour too short for labels
    }
    
    let label_text = config.get_level_label(contour.level);
    let margin = config.label_font_size * 2.0; // Keep labels away from edges
    
    // Calculate how many labels to place
    let num_labels = ((total_length / config.label_spacing).floor() as usize).max(1);
    
    // Space labels evenly along the contour
    let spacing = total_length / (num_labels as f32 + 1.0);
    
    let mut accumulated_length = 0.0;
    let mut next_label_at = spacing;
    let mut label_count = 0;
    
    for i in 1..contour.points.len() {
        if label_count >= num_labels {
            break;
        }
        
        let p1 = contour.points[i - 1];
        let p2 = contour.points[i];
        let dx = p2.x - p1.x;
        let dy = p2.y - p1.y;
        let segment_length = (dx * dx + dy * dy).sqrt();
        
        // Check if this segment contains our next label position
        while accumulated_length + segment_length >= next_label_at && label_count < num_labels {
            let t = (next_label_at - accumulated_length) / segment_length;
            let x = p1.x + t * dx;
            let y = p1.y + t * dy;
            
            // Check if position is within bounds (with margin)
            if x > margin && x < (width as f32 - margin) && 
               y > margin && y < (height as f32 - margin) {
                // Calculate angle from segment direction
                let angle = dy.atan2(dx);
                
                // Flip angle if text would be upside down
                let angle = if angle.abs() > std::f32::consts::FRAC_PI_2 {
                    angle + std::f32::consts::PI
                } else {
                    angle
                };
                
                // Check for overlap with existing labels
                let min_distance = config.label_font_size * 4.0;
                let has_overlap = positions.iter().any(|pos| {
                    let dist_sq = (pos.x - x).powi(2) + (pos.y - y).powi(2);
                    dist_sq < min_distance * min_distance
                });
                
                if !has_overlap {
                    positions.push(LabelPosition {
                        x,
                        y,
                        angle,
                        text: label_text.clone(),
                        level: contour.level,
                    });
                }
            }
            
            next_label_at += spacing;
            label_count += 1;
        }
        
        accumulated_length += segment_length;
    }
}

/// Draw labels on the pixmap
fn draw_labels(
    pixmap: &mut tiny_skia::Pixmap,
    positions: &[LabelPosition],
    config: &ContourConfig,
) {
    // Use simple bitmap font rendering for labels
    // Each character is rendered as a small filled rectangle pattern
    
    for pos in positions {
        let color = config.get_level_color(pos.level);
        draw_text_label(pixmap, pos.x, pos.y, pos.angle, &pos.text, config.label_font_size, color);
    }
}

/// Draw a text label at the given position with rotation
fn draw_text_label(
    pixmap: &mut tiny_skia::Pixmap,
    x: f32,
    y: f32,
    angle: f32,
    text: &str,
    font_size: f32,
    color: [u8; 4],
) {
    use tiny_skia::*;
    
    // Character width and height based on font size
    let char_width = font_size * 0.6;
    let char_height = font_size;
    let char_spacing = font_size * 0.1;
    
    // Calculate total text width
    let text_width = text.len() as f32 * (char_width + char_spacing) - char_spacing;
    
    // Draw background (white with some transparency) for readability
    let bg_padding = font_size * 0.2;
    let bg_width = text_width + bg_padding * 2.0;
    let bg_height = char_height + bg_padding * 2.0;
    
    // Create background rectangle
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    
    // Background paint
    let mut bg_paint = Paint::default();
    bg_paint.set_color_rgba8(255, 255, 255, 220);
    bg_paint.anti_alias = true;
    
    // Draw rotated background rectangle
    let half_w = bg_width / 2.0;
    let half_h = bg_height / 2.0;
    
    // Rectangle corners relative to center
    let corners = [
        (-half_w, -half_h),
        (half_w, -half_h),
        (half_w, half_h),
        (-half_w, half_h),
    ];
    
    // Rotate and translate corners
    let mut pb = PathBuilder::new();
    for (i, (cx, cy)) in corners.iter().enumerate() {
        let rx = cx * cos_a - cy * sin_a + x;
        let ry = cx * sin_a + cy * cos_a + y;
        if i == 0 {
            pb.move_to(rx, ry);
        } else {
            pb.line_to(rx, ry);
        }
    }
    pb.close();
    
    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, &bg_paint, FillRule::Winding, Transform::identity(), None);
    }
    
    // Text paint
    let mut text_paint = Paint::default();
    text_paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    text_paint.anti_alias = true;
    
    // Draw each character using simple shapes
    let start_x = -text_width / 2.0;
    
    for (i, ch) in text.chars().enumerate() {
        let char_x = start_x + i as f32 * (char_width + char_spacing) + char_width / 2.0;
        let char_y = 0.0;
        
        // Rotate character position
        let rx = char_x * cos_a - char_y * sin_a + x;
        let ry = char_x * sin_a + char_y * cos_a + y;
        
        draw_character(pixmap, rx, ry, angle, ch, char_width, char_height, &text_paint);
    }
}

/// Draw a single character as simple geometric shapes
fn draw_character(
    pixmap: &mut tiny_skia::Pixmap,
    x: f32,
    y: f32,
    angle: f32,
    ch: char,
    width: f32,
    height: f32,
    paint: &tiny_skia::Paint,
) {
    use tiny_skia::*;
    
    let cos_a = angle.cos();
    let sin_a = angle.sin();
    let half_w = width / 2.0;
    let half_h = height / 2.0;
    let stroke_w = width * 0.15;
    
    let mut stroke = Stroke::default();
    stroke.width = stroke_w;
    stroke.line_cap = LineCap::Round;
    stroke.line_join = LineJoin::Round;
    
    // Helper to rotate a point around (x, y)
    let rotate = |px: f32, py: f32| -> (f32, f32) {
        (px * cos_a - py * sin_a + x, px * sin_a + py * cos_a + y)
    };
    
    // Define character shapes (simplified 7-segment style)
    let segments: Vec<((f32, f32), (f32, f32))> = match ch {
        '0' => vec![
            ((-half_w, -half_h), (half_w, -half_h)),  // top
            ((half_w, -half_h), (half_w, half_h)),     // right
            ((half_w, half_h), (-half_w, half_h)),     // bottom
            ((-half_w, half_h), (-half_w, -half_h)),   // left
        ],
        '1' => vec![
            ((0.0, -half_h), (0.0, half_h)),           // center vertical
        ],
        '2' => vec![
            ((-half_w, -half_h), (half_w, -half_h)),   // top
            ((half_w, -half_h), (half_w, 0.0)),        // top right
            ((half_w, 0.0), (-half_w, 0.0)),           // middle
            ((-half_w, 0.0), (-half_w, half_h)),       // bottom left
            ((-half_w, half_h), (half_w, half_h)),     // bottom
        ],
        '3' => vec![
            ((-half_w, -half_h), (half_w, -half_h)),   // top
            ((half_w, -half_h), (half_w, half_h)),     // right
            ((half_w, half_h), (-half_w, half_h)),     // bottom
            ((-half_w, 0.0), (half_w, 0.0)),           // middle
        ],
        '4' => vec![
            ((-half_w, -half_h), (-half_w, 0.0)),      // top left
            ((-half_w, 0.0), (half_w, 0.0)),           // middle
            ((half_w, -half_h), (half_w, half_h)),     // right
        ],
        '5' => vec![
            ((half_w, -half_h), (-half_w, -half_h)),   // top
            ((-half_w, -half_h), (-half_w, 0.0)),      // top left
            ((-half_w, 0.0), (half_w, 0.0)),           // middle
            ((half_w, 0.0), (half_w, half_h)),         // bottom right
            ((half_w, half_h), (-half_w, half_h)),     // bottom
        ],
        '6' => vec![
            ((half_w, -half_h), (-half_w, -half_h)),   // top
            ((-half_w, -half_h), (-half_w, half_h)),   // left
            ((-half_w, half_h), (half_w, half_h)),     // bottom
            ((half_w, half_h), (half_w, 0.0)),         // bottom right
            ((half_w, 0.0), (-half_w, 0.0)),           // middle
        ],
        '7' => vec![
            ((-half_w, -half_h), (half_w, -half_h)),   // top
            ((half_w, -half_h), (0.0, half_h)),        // diagonal
        ],
        '8' => vec![
            ((-half_w, -half_h), (half_w, -half_h)),   // top
            ((half_w, -half_h), (half_w, half_h)),     // right
            ((half_w, half_h), (-half_w, half_h)),     // bottom
            ((-half_w, half_h), (-half_w, -half_h)),   // left
            ((-half_w, 0.0), (half_w, 0.0)),           // middle
        ],
        '9' => vec![
            ((-half_w, 0.0), (half_w, 0.0)),           // middle
            ((half_w, 0.0), (half_w, -half_h)),        // top right
            ((half_w, -half_h), (-half_w, -half_h)),   // top
            ((-half_w, -half_h), (-half_w, 0.0)),      // top left
            ((half_w, 0.0), (half_w, half_h)),         // bottom right
        ],
        '-' => vec![
            ((-half_w, 0.0), (half_w, 0.0)),           // middle
        ],
        '.' => vec![
            ((0.0, half_h * 0.7), (0.0, half_h * 0.8)), // dot (small line)
        ],
        _ => vec![], // Unknown character - skip
    };
    
    // Draw each segment
    for ((x1, y1), (x2, y2)) in segments {
        let (rx1, ry1) = rotate(x1, y1);
        let (rx2, ry2) = rotate(x2, y2);
        
        let mut pb = PathBuilder::new();
        pb.move_to(rx1, ry1);
        pb.line_to(rx2, ry2);
        
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, paint, &stroke, Transform::identity(), None);
        }
    }
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
    // TODO: REMOVE THIS - Added for benchmark testing
    std::thread::sleep(std::time::Duration::from_millis(5));

    // Log data stats for debugging
    let valid_data: Vec<f32> = data.iter().filter(|v| !v.is_nan()).copied().collect();
    let data_min = valid_data.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let data_max = valid_data.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    
    tracing::debug!(
        data_len = data.len(),
        valid_count = valid_data.len(),
        data_min = data_min,
        data_max = data_max,
        num_levels = config.levels.len(),
        first_level = config.levels.first().copied().unwrap_or(0.0),
        last_level = config.levels.last().copied().unwrap_or(0.0),
        "render_contours input"
    );
    
    // Generate all contours
    let contours = generate_all_contours(data, width, height, config);
    
    tracing::debug!(
        num_contours = contours.len(),
        total_points = contours.iter().map(|c| c.points.len()).sum::<usize>(),
        "Generated contours"
    );
    
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
