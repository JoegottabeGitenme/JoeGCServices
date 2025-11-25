//! Redis Streams-based job queue for render requests.

use chrono::{DateTime, Utc};
use redis::{aio::MultiplexedConnection, streams::*, AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use wms_common::{BoundingBox, CrsCode, WmsError, WmsResult};

const STREAM_KEY: &str = "render:jobs";
const RESULTS_PREFIX: &str = "render:result:";
const CONSUMER_GROUP: &str = "renderers";

/// Redis Streams job queue for render requests.
pub struct JobQueue {
    conn: MultiplexedConnection,
}

impl JobQueue {
    /// Connect to Redis and initialize the stream.
    pub async fn connect(redis_url: &str) -> WmsResult<Self> {
        let client = Client::open(redis_url)
            .map_err(|e| WmsError::CacheError(format!("Redis connection failed: {}", e)))?;

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| WmsError::CacheError(format!("Redis connection failed: {}", e)))?;

        // Create consumer group if it doesn't exist
        let _: Result<(), _> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(STREAM_KEY)
            .arg(CONSUMER_GROUP)
            .arg("$")
            .arg("MKSTREAM")
            .query_async(&mut conn)
            .await;

        Ok(Self { conn })
    }

    /// Enqueue a render job.
    pub async fn enqueue(&mut self, job: &RenderJob) -> WmsResult<String> {
        let job_json = serde_json::to_string(job)
            .map_err(|e| WmsError::InternalError(format!("Serialization failed: {}", e)))?;

        let entry_id: String = redis::cmd("XADD")
            .arg(STREAM_KEY)
            .arg("*")
            .arg("job_id")
            .arg(&job.id.to_string())
            .arg("data")
            .arg(&job_json)
            .query_async(&mut self.conn)
            .await
            .map_err(|e| WmsError::CacheError(format!("Enqueue failed: {}", e)))?;

        Ok(entry_id)
    }

    /// Claim and return the next available job (for workers).
    pub async fn claim_next(&mut self, consumer_name: &str) -> WmsResult<Option<RenderJob>> {
        let opts = StreamReadOptions::default()
            .group(CONSUMER_GROUP, consumer_name)
            .count(1)
            .block(5000); // 5 second block

        let result: StreamReadReply = self
            .conn
            .xread_options(&[STREAM_KEY], &[">"], &opts)
            .await
            .map_err(|e| WmsError::CacheError(format!("Read failed: {}", e)))?;

        for stream_key in result.keys {
            for entry in stream_key.ids {
                if let Some(data) = entry.map.get("data") {
                    let bytes: Vec<u8> = redis::from_redis_value(data)
                        .map_err(|e| WmsError::InternalError(format!("Parse failed: {}", e)))?;
                    let job: RenderJob = serde_json::from_slice(&bytes).map_err(|e| {
                        WmsError::InternalError(format!("Deserialize failed: {}", e))
                    })?;
                    return Ok(Some(job));
                }
            }
        }

        Ok(None)
    }

    /// Mark a job as completed and store the result.
    pub async fn complete(&mut self, job_id: &Uuid, result: &[u8]) -> WmsResult<()> {
        let result_key = format!("{}{}", RESULTS_PREFIX, job_id);

        // Store result with 5 minute expiry
        self.conn
            .set_ex(&result_key, result, 300)
            .await
            .map_err(|e| WmsError::CacheError(format!("Store result failed: {}", e)))?;

        // Publish completion notification
        self.conn
            .publish("render:complete", job_id.to_string())
            .await
            .map_err(|e| WmsError::CacheError(format!("Publish failed: {}", e)))?;

        Ok(())
    }

    /// Mark a job as failed.
    pub async fn fail(&mut self, job_id: &Uuid, error: &str) -> WmsResult<()> {
        let result_key = format!("{}{}", RESULTS_PREFIX, job_id);
        let error_data = serde_json::json!({ "error": error }).to_string();

        self.conn
            .set_ex(&result_key, error_data.as_bytes(), 300)
            .await
            .map_err(|e| WmsError::CacheError(format!("Store error failed: {}", e)))?;

        self.conn
            .publish("render:complete", job_id.to_string())
            .await
            .map_err(|e| WmsError::CacheError(format!("Publish failed: {}", e)))?;

        Ok(())
    }

    /// Wait for a job result.
    pub async fn wait_for_result(
        &mut self,
        job_id: &Uuid,
        timeout_ms: u64,
    ) -> WmsResult<JobResult> {
        let result_key = format!("{}{}", RESULTS_PREFIX, job_id);

        // Poll for result (simple implementation)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        loop {
            let result: Option<Vec<u8>> = self
                .conn
                .get(&result_key)
                .await
                .map_err(|e| WmsError::CacheError(format!("Get result failed: {}", e)))?;

            if let Some(data) = result {
                // Check if it's an error
                if let Ok(error) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(err_msg) = error.get("error").and_then(|e| e.as_str()) {
                        return Ok(JobResult::Failed(err_msg.to_string()));
                    }
                }
                return Ok(JobResult::Success(data.into()));
            }

            if start.elapsed() > timeout {
                return Ok(JobResult::Timeout);
            }

            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Get queue depth (pending jobs).
    pub async fn queue_depth(&mut self) -> WmsResult<u64> {
        let info: StreamInfoStreamReply = self
            .conn
            .xinfo_stream(STREAM_KEY)
            .await
            .map_err(|e| WmsError::CacheError(format!("XINFO failed: {}", e)))?;

        Ok(info.length as u64)
    }
}

/// A render job request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderJob {
    pub id: Uuid,
    pub layer: String,
    pub style: String,
    pub crs: CrsCode,
    pub bbox: BoundingBox,
    pub width: u32,
    pub height: u32,
    pub time: Option<String>,
    pub format: String,
    pub created_at: DateTime<Utc>,
    pub priority: JobPriority,
}

impl RenderJob {
    pub fn new(
        layer: impl Into<String>,
        style: impl Into<String>,
        crs: CrsCode,
        bbox: BoundingBox,
        width: u32,
        height: u32,
        time: Option<String>,
        format: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            layer: layer.into(),
            style: style.into(),
            crs,
            bbox,
            width,
            height,
            time,
            format: format.into(),
            created_at: Utc::now(),
            priority: JobPriority::Normal,
        }
    }

    pub fn with_priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }
}

/// Job priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobPriority {
    Low,
    Normal,
    High,
}

/// Job execution result.
#[derive(Debug)]
pub enum JobResult {
    Success(bytes::Bytes),
    Failed(String),
    Timeout,
}

/// Job status for monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    Pending,
    Processing,
    Completed,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_job_serialization() {
        let job = RenderJob::new(
            "gfs:temperature_2m",
            "gradient",
            CrsCode::Epsg3857,
            BoundingBox::new(-125.0, 24.0, -66.0, 50.0),
            512,
            512,
            Some("2024-01-15T12:00:00Z".to_string()),
            "png",
        );

        let json = serde_json::to_string(&job).unwrap();
        let parsed: RenderJob = serde_json::from_str(&json).unwrap();

        assert_eq!(job.id, parsed.id);
        assert_eq!(job.layer, parsed.layer);
    }
}
