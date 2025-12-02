//! Tile request URL generation.

use crate::config::{BBox, TestConfig, TileSelection, TimeSelection, TimeOrder};
use crate::wms_client;
use rand::prelude::*;
use std::f64::consts::PI;

/// Generates WMTS tile request URLs.
pub struct TileGenerator {
    config: TestConfig,
    rng: StdRng,
    _layer_weights: Vec<f64>,
    layer_cumulative: Vec<f64>,
    time_index: usize,  // For sequential time selection
    resolved_times: Option<Vec<String>>,  // Times resolved from WMS queries
}

impl TileGenerator {
    /// Create a new tile generator.
    /// For QuerySequential/QueryRandom time selections, this will fetch times from WMS.
    pub async fn new_async(config: TestConfig) -> anyhow::Result<Self> {
        // Resolve times if needed
        let resolved_times = match &config.time_selection {
            Some(TimeSelection::QuerySequential { layer, count, order }) => {
                let mut times = wms_client::query_layer_times(&config.base_url, layer).await?;
                
                // Order times (WMS returns newest first typically)
                match order {
                    TimeOrder::NewestFirst => {
                        // Already in newest-first order
                    }
                    TimeOrder::OldestFirst => {
                        times.reverse();
                    }
                }
                
                // TODO: Warn user if fewer times available than requested
                // This avoids test failures when the system has fewer timesteps ingested
                // than the scenario expects. Currently we silently use whatever is available.
                // Example: Scenario expects 5 times but only 2 are ingested - test should
                // proceed with 2 times and log a warning rather than failing.
                if times.len() < *count {
                    eprintln!(
                        "Warning: Only {} time(s) available for layer '{}', but {} requested. Using all available times.",
                        times.len(), layer, count
                    );
                }
                
                // Take the requested count (or all available if fewer)
                times.truncate(*count);
                Some(times)
            }
            Some(TimeSelection::QueryRandom { layer, count, order }) => {
                let mut times = wms_client::query_layer_times(&config.base_url, layer).await?;
                
                // Order times
                match order {
                    TimeOrder::NewestFirst => {
                        // Already in newest-first order
                    }
                    TimeOrder::OldestFirst => {
                        times.reverse();
                    }
                }
                
                // TODO: Warn user if fewer times available than requested
                // This avoids test failures when the system has fewer timesteps ingested
                // than the scenario expects. Currently we silently use whatever is available.
                if times.len() < *count {
                    eprintln!(
                        "Warning: Only {} time(s) available for layer '{}', but {} requested. Using all available times.",
                        times.len(), layer, count
                    );
                }
                
                // Take the requested count for the pool to randomly select from (or all available if fewer)
                times.truncate(*count);
                Some(times)
            }
            _ => None,
        };
        
        Ok(Self::new_with_times(config, resolved_times))
    }
    
    /// Create a new tile generator (synchronous version).
    pub fn new(config: TestConfig) -> Self {
        Self::new_with_times(config, None)
    }
    
    /// Create a new tile generator with pre-resolved times.
    fn new_with_times(config: TestConfig, resolved_times: Option<Vec<String>>) -> Self {
        // Calculate cumulative weights for weighted random layer selection
        let mut weights: Vec<f64> = config.layers.iter().map(|l| l.weight).collect();
        let total: f64 = weights.iter().sum();
        
        // Normalize weights
        for w in &mut weights {
            *w /= total;
        }
        
        // Calculate cumulative distribution
        let mut cumulative = Vec::with_capacity(weights.len());
        let mut sum = 0.0;
        for w in &weights {
            sum += w;
            cumulative.push(sum);
        }
        
        // Use seed if provided for reproducible tests, otherwise use entropy
        let rng = if let Some(seed) = config.seed {
            StdRng::seed_from_u64(seed)
        } else {
            StdRng::from_entropy()
        };
        
        Self {
            config,
            rng,
            _layer_weights: weights,
            layer_cumulative: cumulative,
            time_index: 0,
            resolved_times,
        }
    }

    /// Generate the next tile request URL.
    pub fn next_url(&mut self) -> String {
        self.next_url_with_info().0
    }
    
    /// Generate the next tile request URL with tile info for logging.
    /// Returns (url, (z, x, y, layer_name))
    pub fn next_url_with_info(&mut self) -> (String, (u32, u32, u32, String)) {
        // Select layer index based on weights
        let layer_idx = self.select_layer_index();
        
        // Generate tile coordinates
        let (z, x, y) = self.generate_tile_coords();
        
        // Select time (if temporal testing enabled)
        let time = self.select_time();
        
        // Get layer name
        let layer_name = self.config.layers[layer_idx].name.clone();
        
        // Build WMTS URL
        let url = self.build_wmts_url(layer_idx, z, x, y, time.as_deref());
        
        (url, (z, x, y, layer_name))
    }

    /// Select a layer index based on configured weights.
    fn select_layer_index(&mut self) -> usize {
        let r: f64 = self.rng.gen();
        
        for (i, &cum) in self.layer_cumulative.iter().enumerate() {
            if r <= cum {
                return i;
            }
        }
        
        // Fallback (shouldn't happen)
        0
    }

    /// Generate tile coordinates based on selection strategy.
    fn generate_tile_coords(&mut self) -> (u32, u32, u32) {
        match &self.config.tile_selection {
            TileSelection::Random { zoom_range, bbox } => {
                let (min_z, max_z) = *zoom_range;
                let z = self.rng.gen_range(min_z..=max_z);
                
                if let Some(bbox) = bbox {
                    // Generate random tile within bbox
                    let tiles = Self::tiles_in_bbox(bbox, z);
                    if !tiles.is_empty() {
                        let idx = self.rng.gen_range(0..tiles.len());
                        let (x, y) = tiles[idx];
                        (z, x, y)
                    } else {
                        // Fallback to global random
                        let max = Self::max_tile_for_zoom(z);
                        let x = self.rng.gen_range(0..=max);
                        let y = self.rng.gen_range(0..=max);
                        (z, x, y)
                    }
                } else {
                    // Generate random tile globally
                    let max = Self::max_tile_for_zoom(z);
                    let x = self.rng.gen_range(0..=max);
                    let y = self.rng.gen_range(0..=max);
                    (z, x, y)
                }
            }
            TileSelection::Sequential { zoom, bbox } => {
                // TODO: Implement sequential iteration through tiles
                // For now, use first tile in bbox
                let tiles = Self::tiles_in_bbox(bbox, *zoom);
                if !tiles.is_empty() {
                    let (x, y) = tiles[0];
                    (*zoom, x, y)
                } else {
                    (*zoom, 0, 0)
                }
            }
            TileSelection::Fixed { tiles } => {
                // Pick random from fixed list
                if !tiles.is_empty() {
                    let idx = self.rng.gen_range(0..tiles.len());
                    tiles[idx]
                } else {
                    (0, 0, 0)
                }
            }
            TileSelection::PanSimulation { start, steps: _ } => {
                // TODO: Implement pan simulation
                // For now, just use start tile
                *start
            }
        }
    }

    /// Select a time value based on configured time selection strategy.
    fn select_time(&mut self) -> Option<String> {
        match &self.config.time_selection {
            Some(TimeSelection::Sequential { times }) => {
                if times.is_empty() {
                    return None;
                }
                let time = times[self.time_index % times.len()].clone();
                self.time_index += 1;
                Some(time)
            }
            Some(TimeSelection::Random { times }) => {
                if times.is_empty() {
                    return None;
                }
                let idx = self.rng.gen_range(0..times.len());
                Some(times[idx].clone())
            }
            Some(TimeSelection::QuerySequential { .. }) => {
                // Use resolved times sequentially
                if let Some(times) = &self.resolved_times {
                    if times.is_empty() {
                        return None;
                    }
                    let time = times[self.time_index % times.len()].clone();
                    self.time_index += 1;
                    Some(time)
                } else {
                    None
                }
            }
            Some(TimeSelection::QueryRandom { .. }) => {
                // Use resolved times randomly
                if let Some(times) = &self.resolved_times {
                    if times.is_empty() {
                        return None;
                    }
                    let idx = self.rng.gen_range(0..times.len());
                    Some(times[idx].clone())
                } else {
                    None
                }
            }
            Some(TimeSelection::None) | None => None,
        }
    }

    /// Build a WMTS GetTile URL.
    fn build_wmts_url(&self, layer_idx: usize, z: u32, x: u32, y: u32, time: Option<&str>) -> String {
        let layer = &self.config.layers[layer_idx];
        let style = layer.style.as_deref().unwrap_or("default");
        
        let mut url = format!(
            "{}/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER={}&STYLE={}&FORMAT=image/png&TILEMATRIXSET=WebMercatorQuad&TILEMATRIX={}&TILEROW={}&TILECOL={}",
            self.config.base_url,
            layer.name,
            style,
            z,
            y,
            x
        );
        
        // Add TIME parameter if provided
        if let Some(t) = time {
            url.push_str(&format!("&TIME={}", t));
        }
        
        url
    }

    /// Convert lat/lon to tile coordinates at a given zoom level.
    pub fn latlon_to_tile(lat: f64, lon: f64, zoom: u32) -> (u32, u32) {
        let n = 2u32.pow(zoom) as f64;
        let x = ((lon + 180.0) / 360.0 * n).floor() as u32;
        let lat_rad = lat.to_radians();
        let y = ((1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / PI) / 2.0 * n).floor() as u32;
        (x, y)
    }

    /// Get the maximum tile coordinate for a zoom level.
    pub fn max_tile_for_zoom(zoom: u32) -> u32 {
        2u32.pow(zoom) - 1
    }

    /// Get all tiles within a bounding box at a given zoom level.
    pub fn tiles_in_bbox(bbox: &BBox, zoom: u32) -> Vec<(u32, u32)> {
        // Convert bbox corners to tile coordinates
        // Note: latitude order is swapped because tile Y increases southward
        let (x_min, y_max) = Self::latlon_to_tile(bbox.min_lat, bbox.min_lon, zoom);
        let (x_max, y_min) = Self::latlon_to_tile(bbox.max_lat, bbox.max_lon, zoom);

        // Clamp to valid tile range for this zoom level
        let max_tile = Self::max_tile_for_zoom(zoom);
        let x_min = x_min.min(max_tile);
        let x_max = x_max.min(max_tile);
        let y_min = y_min.min(max_tile);
        let y_max = y_max.min(max_tile);

        let mut tiles = Vec::new();
        for x in x_min..=x_max {
            for y in y_min..=y_max {
                tiles.push((x, y));
            }
        }
        tiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latlon_to_tile() {
        // Test center of map (0, 0) at zoom 0
        let (x, y) = TileGenerator::latlon_to_tile(0.0, 0.0, 0);
        assert_eq!((x, y), (0, 0));

        // Test New York (~40.7, -74.0) at zoom 10
        let (x, y) = TileGenerator::latlon_to_tile(40.7, -74.0, 10);
        assert!(x < 2u32.pow(10));
        assert!(y < 2u32.pow(10));
    }

    #[test]
    fn test_max_tile_for_zoom() {
        assert_eq!(TileGenerator::max_tile_for_zoom(0), 0);
        assert_eq!(TileGenerator::max_tile_for_zoom(1), 1);
        assert_eq!(TileGenerator::max_tile_for_zoom(10), 1023);
    }

    #[test]
    fn test_tiles_in_bbox() {
        let bbox = BBox {
            min_lon: -180.0,
            min_lat: -85.0,
            max_lon: 180.0,
            max_lat: 85.0,
        };
        
        let tiles = TileGenerator::tiles_in_bbox(&bbox, 0);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0], (0, 0));
    }
}
