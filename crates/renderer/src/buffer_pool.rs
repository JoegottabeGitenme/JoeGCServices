//! Thread-local buffer pools for reducing allocation overhead.
//!
//! This module provides reusable buffers for the rendering pipeline. Instead of
//! allocating fresh `Vec`s for each tile render, buffers are cached per-thread
//! and reused across requests.
//!
//! ## Design
//!
//! - **Thread-local storage**: Each thread has its own buffer cache, avoiding
//!   contention in async/multi-threaded environments.
//! - **Tiered sizing**: Buffers are sized for common tile dimensions (256, 512, 1024)
//!   to minimize resizing.
//! - **Automatic clearing**: Buffers are cleared before reuse to ensure transparency.
//!
//! ## Usage
//!
//! ```ignore
//! use renderer::buffer_pool::{with_pixel_buffer, with_index_buffer, with_resample_buffer};
//!
//! // Get a pixel buffer, use it, result is returned
//! let png = with_pixel_buffer(256, 256, |pixels| {
//!     // Fill pixels...
//!     create_png(pixels, 256, 256)
//! })?;
//!
//! // Get an index buffer for indexed PNG rendering
//! let png = with_index_buffer(256, 256, |indices| {
//!     // Fill indices...
//!     create_png_indexed(256, 256, &palette, indices)
//! })?;
//! ```
//!
//! ## Performance Impact
//!
//! Buffer pooling primarily improves p99 latency under high load by reducing
//! allocator contention. For single requests, the benefit is minimal since
//! allocators are highly optimized for common sizes.

use std::cell::RefCell;

/// Standard tile sizes for pre-allocated buffers
const TILE_256: usize = 256 * 256;
const TILE_512: usize = 512 * 512;
const TILE_1024: usize = 1024 * 1024;

// Thread-local pixel buffer (RGBA, 4 bytes per pixel)
thread_local! {
    static PIXEL_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(TILE_256 * 4));
}

// Thread-local index buffer (1 byte per pixel for indexed PNG)
thread_local! {
    static INDEX_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(TILE_256));
}

// Thread-local resample buffer (f32 per pixel)
thread_local! {
    static RESAMPLE_BUFFER: RefCell<Vec<f32>> = RefCell::new(Vec::with_capacity(TILE_256));
}

// Thread-local PNG output buffer
thread_local! {
    static PNG_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(TILE_256)); // PNG is compressed
}

// Thread-local scanline buffer for PNG encoding
thread_local! {
    static SCANLINE_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(TILE_256 + 256)); // +filter bytes
}

/// Get a reusable RGBA pixel buffer.
///
/// The buffer is resized to `width * height * 4` and filled with zeros (transparent).
/// The closure receives a mutable slice of the exact required size.
///
/// # Arguments
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
/// * `f` - Closure that uses the buffer and returns a result
///
/// # Returns
/// The result of the closure
#[inline]
pub fn with_pixel_buffer<F, R>(width: usize, height: usize, f: F) -> R
where
    F: FnOnce(&mut [u8]) -> R,
{
    PIXEL_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height * 4;
        
        // Resize if needed (Vec::resize is efficient for growing)
        if buf.len() < size {
            buf.resize(size, 0);
        }
        
        // Clear to transparent (zero) - required for proper alpha handling
        // Only clear the portion we'll use
        buf[..size].fill(0);
        
        f(&mut buf[..size])
    })
}

/// Get a reusable RGBA pixel buffer, returning owned Vec.
///
/// This variant returns an owned Vec for APIs that require ownership.
/// The buffer contents are moved out and replaced with a new allocation.
///
/// # Arguments
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
/// * `f` - Closure that fills the buffer
///
/// # Returns
/// Owned Vec<u8> with the filled pixel data
#[inline]
pub fn take_pixel_buffer<F>(width: usize, height: usize, f: F) -> Vec<u8>
where
    F: FnOnce(&mut [u8]),
{
    PIXEL_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height * 4;
        
        // Ensure capacity - compute len before mutable borrow
        let current_len = buf.len();
        let current_cap = buf.capacity();
        if current_cap < size {
            buf.reserve(size - current_len);
        }
        buf.resize(size, 0);
        buf[..size].fill(0);
        
        // Fill the buffer
        f(&mut buf[..size]);
        
        // Take the buffer and replace with a new one
        // The old buffer becomes the return value
        std::mem::replace(&mut *buf, Vec::with_capacity(optimal_capacity(size)))
    })
}

/// Get a reusable index buffer for indexed PNG rendering.
///
/// The buffer is resized to `width * height` (1 byte per pixel).
///
/// # Arguments
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
/// * `f` - Closure that uses the buffer and returns a result
///
/// # Returns
/// The result of the closure
#[inline]
pub fn with_index_buffer<F, R>(width: usize, height: usize, f: F) -> R
where
    F: FnOnce(&mut [u8]) -> R,
{
    INDEX_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height;
        
        if buf.len() < size {
            buf.resize(size, 0);
        }
        buf[..size].fill(0);
        
        f(&mut buf[..size])
    })
}

/// Get a reusable index buffer, returning owned Vec.
#[inline]
pub fn take_index_buffer<F>(width: usize, height: usize, f: F) -> Vec<u8>
where
    F: FnOnce(&mut [u8]),
{
    INDEX_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height;
        
        let current_len = buf.len();
        let current_cap = buf.capacity();
        if current_cap < size {
            buf.reserve(size - current_len);
        }
        buf.resize(size, 0);
        buf[..size].fill(0);
        
        f(&mut buf[..size]);
        
        std::mem::replace(&mut *buf, Vec::with_capacity(optimal_capacity(size)))
    })
}

/// Get a reusable f32 buffer for resampling operations.
///
/// The buffer is resized to `width * height` and filled with zeros.
///
/// # Arguments
/// * `width` - Grid width
/// * `height` - Grid height
/// * `f` - Closure that uses the buffer and returns a result
///
/// # Returns
/// The result of the closure
#[inline]
pub fn with_resample_buffer<F, R>(width: usize, height: usize, f: F) -> R
where
    F: FnOnce(&mut [f32]) -> R,
{
    RESAMPLE_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height;
        
        if buf.len() < size {
            buf.resize(size, 0.0);
        }
        buf[..size].fill(0.0);
        
        f(&mut buf[..size])
    })
}

/// Get a reusable f32 buffer, returning owned Vec.
#[inline]
pub fn take_resample_buffer<F>(width: usize, height: usize, f: F) -> Vec<f32>
where
    F: FnOnce(&mut [f32]),
{
    RESAMPLE_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        let size = width * height;
        
        let current_len = buf.len();
        let current_cap = buf.capacity();
        if current_cap < size {
            buf.reserve(size - current_len);
        }
        buf.resize(size, 0.0);
        buf[..size].fill(0.0);
        
        f(&mut buf[..size]);
        
        std::mem::replace(&mut *buf, Vec::with_capacity(optimal_capacity(size)))
    })
}

/// Get a reusable PNG output buffer.
///
/// Used for building the final PNG byte stream.
#[inline]
pub fn with_png_buffer<F, R>(estimated_size: usize, f: F) -> R
where
    F: FnOnce(&mut Vec<u8>) -> R,
{
    PNG_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        
        let current_cap = buf.capacity();
        if current_cap < estimated_size {
            buf.reserve(estimated_size - current_cap);
        }
        
        f(&mut buf)
    })
}

/// Get a reusable scanline buffer for PNG encoding.
///
/// Used for building uncompressed scanline data before deflate.
#[inline]
pub fn with_scanline_buffer<F, R>(width: usize, height: usize, bytes_per_pixel: usize, f: F) -> R
where
    F: FnOnce(&mut Vec<u8>) -> R,
{
    SCANLINE_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        
        // Each scanline: 1 filter byte + width * bytes_per_pixel
        let size = height * (1 + width * bytes_per_pixel);
        let current_cap = buf.capacity();
        if current_cap < size {
            buf.reserve(size - current_cap);
        }
        
        f(&mut buf)
    })
}

/// Return an optimal pre-allocation capacity for the given size.
/// Rounds up to common tile sizes to reduce future reallocations.
#[inline]
fn optimal_capacity(size: usize) -> usize {
    if size <= TILE_256 {
        TILE_256
    } else if size <= TILE_512 {
        TILE_512
    } else if size <= TILE_1024 {
        TILE_1024
    } else {
        // Round up to next power of 2 for very large buffers
        size.next_power_of_two()
    }
}

/// Statistics about buffer pool usage (for debugging/monitoring)
#[derive(Debug, Default, Clone)]
pub struct PoolStats {
    pub pixel_buffer_capacity: usize,
    pub index_buffer_capacity: usize,
    pub resample_buffer_capacity: usize,
    pub png_buffer_capacity: usize,
}

/// Get current buffer pool statistics for this thread.
pub fn get_pool_stats() -> PoolStats {
    PoolStats {
        pixel_buffer_capacity: PIXEL_BUFFER.with(|b| b.borrow().capacity()),
        index_buffer_capacity: INDEX_BUFFER.with(|b| b.borrow().capacity()),
        resample_buffer_capacity: RESAMPLE_BUFFER.with(|b| b.borrow().capacity()),
        png_buffer_capacity: PNG_BUFFER.with(|b| b.borrow().capacity()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_buffer_reuse() {
        // First use - should allocate
        let result1 = with_pixel_buffer(256, 256, |buf| {
            assert_eq!(buf.len(), 256 * 256 * 4);
            buf[0] = 255;
            buf[0]
        });
        assert_eq!(result1, 255);
        
        // Second use - should reuse (buffer is cleared)
        let result2 = with_pixel_buffer(256, 256, |buf| {
            // Should be cleared
            assert_eq!(buf[0], 0);
            buf.len()
        });
        assert_eq!(result2, 256 * 256 * 4);
    }

    #[test]
    fn test_pixel_buffer_resize() {
        // Small buffer first
        with_pixel_buffer(64, 64, |buf| {
            assert_eq!(buf.len(), 64 * 64 * 4);
        });
        
        // Larger buffer - should resize
        with_pixel_buffer(512, 512, |buf| {
            assert_eq!(buf.len(), 512 * 512 * 4);
        });
        
        // Small buffer again - uses subset of large buffer
        with_pixel_buffer(64, 64, |buf| {
            assert_eq!(buf.len(), 64 * 64 * 4);
        });
    }

    #[test]
    fn test_index_buffer() {
        let result = with_index_buffer(256, 256, |buf| {
            assert_eq!(buf.len(), 256 * 256);
            buf[100] = 42;
            buf[100]
        });
        assert_eq!(result, 42);
        
        // Should be cleared on next use
        with_index_buffer(256, 256, |buf| {
            assert_eq!(buf[100], 0);
        });
    }

    #[test]
    fn test_resample_buffer() {
        let result = with_resample_buffer(256, 256, |buf| {
            assert_eq!(buf.len(), 256 * 256);
            buf[0] = 3.14;
            buf[0]
        });
        assert!((result - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_take_pixel_buffer() {
        let vec = take_pixel_buffer(256, 256, |buf| {
            buf[0] = 255;
            buf[1] = 128;
        });
        
        assert_eq!(vec.len(), 256 * 256 * 4);
        assert_eq!(vec[0], 255);
        assert_eq!(vec[1], 128);
    }

    #[test]
    fn test_pool_stats() {
        // Trigger some allocations
        with_pixel_buffer(512, 512, |_| {});
        with_index_buffer(256, 256, |_| {});
        
        let stats = get_pool_stats();
        assert!(stats.pixel_buffer_capacity >= 512 * 512 * 4);
        assert!(stats.index_buffer_capacity >= 256 * 256);
    }

    #[test]
    fn test_optimal_capacity() {
        assert_eq!(optimal_capacity(100), TILE_256);
        assert_eq!(optimal_capacity(TILE_256), TILE_256);
        assert_eq!(optimal_capacity(TILE_256 + 1), TILE_512);
        assert_eq!(optimal_capacity(TILE_512 + 1), TILE_1024);
    }
}
