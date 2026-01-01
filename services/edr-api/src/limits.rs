//! Response size limit calculation.

use crate::config::LimitsConfig;

/// Estimated response size for a query.
#[derive(Debug, Clone)]
pub struct ResponseSizeEstimate {
    /// Number of parameters requested.
    pub num_parameters: usize,

    /// Number of time steps.
    pub num_time_steps: usize,

    /// Number of vertical levels.
    pub num_vertical_levels: usize,

    /// Number of spatial points (1 for position query).
    pub num_points: usize,

    /// Estimated size in bytes.
    pub estimated_bytes: usize,
}

impl ResponseSizeEstimate {
    /// Estimate the response size for a position query.
    pub fn for_position(
        num_parameters: usize,
        num_time_steps: usize,
        num_vertical_levels: usize,
    ) -> Self {
        // Each data point is ~4 bytes (f32)
        // JSON overhead is roughly 100 bytes per value
        let num_points = 1;
        let data_bytes = num_parameters * num_time_steps * num_vertical_levels * num_points * 4;
        let json_overhead = num_parameters * 200 + num_time_steps * 50 + num_vertical_levels * 20;
        let base_overhead = 1000; // Domain, referencing, etc.

        Self {
            num_parameters,
            num_time_steps,
            num_vertical_levels,
            num_points,
            estimated_bytes: data_bytes + json_overhead + base_overhead,
        }
    }

    /// Estimate the response size for an area query.
    pub fn for_area(
        num_parameters: usize,
        num_time_steps: usize,
        num_vertical_levels: usize,
        bbox_area_sq_degrees: f64,
        resolution_degrees: f64,
    ) -> Self {
        // Estimate number of grid points in the area
        let num_points =
            (bbox_area_sq_degrees / (resolution_degrees * resolution_degrees)) as usize;
        let num_points = num_points.max(1);

        let data_bytes = num_parameters * num_time_steps * num_vertical_levels * num_points * 4;
        let json_overhead = num_parameters * 200 + num_points * 10;
        let base_overhead = 1000;

        Self {
            num_parameters,
            num_time_steps,
            num_vertical_levels,
            num_points,
            estimated_bytes: data_bytes + json_overhead + base_overhead,
        }
    }

    /// Estimate the response size for a radius query.
    ///
    /// Radius queries are similar to area queries but the area is circular.
    /// The bounding box area is π/4 * (2r)² = πr² (circle inscribed in bbox).
    pub fn for_radius(
        num_parameters: usize,
        num_time_steps: usize,
        num_vertical_levels: usize,
        radius_km: f64,
        resolution_degrees: f64,
    ) -> Self {
        // Convert radius to degrees (approximate)
        // At mid-latitudes, 1 degree ≈ 100 km
        let radius_degrees = radius_km / 100.0;

        // Bounding box is 2r x 2r, but only π/4 of that is actually the circle
        let bbox_side = 2.0 * radius_degrees;
        let bbox_area = bbox_side * bbox_side;
        let circle_area = std::f64::consts::PI / 4.0 * bbox_area;

        // Estimate number of grid points in the circular area
        let num_points = (circle_area / (resolution_degrees * resolution_degrees)) as usize;
        let num_points = num_points.max(1);

        let data_bytes = num_parameters * num_time_steps * num_vertical_levels * num_points * 4;
        let json_overhead = num_parameters * 200 + num_points * 10;
        let base_overhead = 1000;

        Self {
            num_parameters,
            num_time_steps,
            num_vertical_levels,
            num_points,
            estimated_bytes: data_bytes + json_overhead + base_overhead,
        }
    }

    /// Estimate the response size for a trajectory query.
    ///
    /// Trajectory queries sample data at each waypoint along the path.
    /// The number of points is simply the number of waypoints.
    pub fn for_trajectory(
        num_parameters: usize,
        num_waypoints: usize,
        num_time_steps: usize,
        num_vertical_levels: usize,
    ) -> Self {
        // For trajectory, num_points = num_waypoints
        // If time is embedded in coords (LINESTRINGM), num_time_steps should be 1
        // since each waypoint has its own time
        let num_points = num_waypoints;

        let data_bytes = num_parameters * num_time_steps * num_vertical_levels * num_points * 4;
        let json_overhead = num_parameters * 200 + num_points * 50; // More overhead per point for trajectory
        let base_overhead = 1500; // Slightly more for trajectory domain

        Self {
            num_parameters,
            num_time_steps,
            num_vertical_levels,
            num_points,
            estimated_bytes: data_bytes + json_overhead + base_overhead,
        }
    }

    /// Get estimated size in megabytes.
    pub fn estimated_mb(&self) -> f64 {
        self.estimated_bytes as f64 / (1024.0 * 1024.0)
    }

    /// Check if this estimate exceeds the limits.
    pub fn check_limits(&self, limits: &LimitsConfig) -> Result<(), LimitExceeded> {
        if self.num_parameters > limits.max_parameters_per_request {
            return Err(LimitExceeded::TooManyParameters {
                requested: self.num_parameters,
                limit: limits.max_parameters_per_request,
            });
        }

        if self.num_time_steps > limits.max_time_steps {
            return Err(LimitExceeded::TooManyTimeSteps {
                requested: self.num_time_steps,
                limit: limits.max_time_steps,
            });
        }

        if self.num_vertical_levels > limits.max_vertical_levels {
            return Err(LimitExceeded::TooManyLevels {
                requested: self.num_vertical_levels,
                limit: limits.max_vertical_levels,
            });
        }

        let max_bytes = limits.max_response_size_mb * 1024 * 1024;
        if self.estimated_bytes > max_bytes {
            return Err(LimitExceeded::ResponseTooLarge {
                estimated_mb: self.estimated_mb(),
                limit_mb: limits.max_response_size_mb,
            });
        }

        Ok(())
    }
}

/// Limit exceeded error.
#[derive(Debug, Clone)]
pub enum LimitExceeded {
    TooManyParameters { requested: usize, limit: usize },
    TooManyTimeSteps { requested: usize, limit: usize },
    TooManyLevels { requested: usize, limit: usize },
    ResponseTooLarge { estimated_mb: f64, limit_mb: usize },
}

impl std::fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitExceeded::TooManyParameters { requested, limit } => {
                write!(
                    f,
                    "Too many parameters: {} requested, limit is {}",
                    requested, limit
                )
            }
            LimitExceeded::TooManyTimeSteps { requested, limit } => {
                write!(
                    f,
                    "Too many time steps: {} requested, limit is {}",
                    requested, limit
                )
            }
            LimitExceeded::TooManyLevels { requested, limit } => {
                write!(
                    f,
                    "Too many vertical levels: {} requested, limit is {}",
                    requested, limit
                )
            }
            LimitExceeded::ResponseTooLarge {
                estimated_mb,
                limit_mb,
            } => {
                write!(
                    f,
                    "Response too large: estimated {:.1}MB, limit is {}MB",
                    estimated_mb, limit_mb
                )
            }
        }
    }
}

impl std::error::Error for LimitExceeded {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_estimate() {
        let estimate = ResponseSizeEstimate::for_position(3, 24, 6);

        assert_eq!(estimate.num_parameters, 3);
        assert_eq!(estimate.num_time_steps, 24);
        assert_eq!(estimate.num_vertical_levels, 6);
        assert_eq!(estimate.num_points, 1);
        assert!(estimate.estimated_bytes > 0);
    }

    #[test]
    fn test_area_estimate() {
        // 10x10 degree area at 0.03 degree resolution = ~111k points
        let estimate = ResponseSizeEstimate::for_area(1, 1, 1, 100.0, 0.03);

        assert!(estimate.num_points > 100000);
        assert!(estimate.estimated_mb() > 0.0);
    }

    #[test]
    fn test_check_limits_ok() {
        let estimate = ResponseSizeEstimate::for_position(3, 24, 6);
        let limits = LimitsConfig::default();

        assert!(estimate.check_limits(&limits).is_ok());
    }

    #[test]
    fn test_check_limits_too_many_params() {
        let estimate = ResponseSizeEstimate::for_position(20, 1, 1);
        let limits = LimitsConfig {
            max_parameters_per_request: 10,
            ..Default::default()
        };

        let result = estimate.check_limits(&limits);
        assert!(matches!(
            result,
            Err(LimitExceeded::TooManyParameters { .. })
        ));
    }

    #[test]
    fn test_check_limits_response_too_large() {
        // Very large area query
        let estimate = ResponseSizeEstimate::for_area(10, 48, 20, 10000.0, 0.03);
        let limits = LimitsConfig {
            max_response_size_mb: 50,
            ..Default::default()
        };

        let result = estimate.check_limits(&limits);
        assert!(matches!(
            result,
            Err(LimitExceeded::ResponseTooLarge { .. })
        ));
    }

    #[test]
    fn test_limit_exceeded_display() {
        let err = LimitExceeded::TooManyParameters {
            requested: 20,
            limit: 10,
        };
        let display = format!("{}", err);
        assert!(display.contains("20"));
        assert!(display.contains("10"));
    }

    #[test]
    fn test_radius_estimate() {
        // 100 km radius at 0.03 degree resolution
        let estimate = ResponseSizeEstimate::for_radius(1, 1, 1, 100.0, 0.03);

        // 100km ≈ 1 degree, so area ≈ π square degrees
        // At 0.03 degree resolution, that's roughly π / 0.0009 ≈ 3500 points
        assert!(estimate.num_points > 1000);
        assert!(estimate.estimated_mb() > 0.0);
    }

    #[test]
    fn test_trajectory_estimate() {
        // Trajectory with 100 waypoints, 3 parameters
        let estimate = ResponseSizeEstimate::for_trajectory(3, 100, 1, 1);

        assert_eq!(estimate.num_points, 100);
        assert_eq!(estimate.num_parameters, 3);
        assert!(estimate.estimated_bytes > 0);
        assert!(estimate.estimated_mb() > 0.0);
    }
}
