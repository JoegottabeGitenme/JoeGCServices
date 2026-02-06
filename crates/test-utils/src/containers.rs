//! Docker container utilities for integration testing.
//!
//! This module provides utilities for spinning up test infrastructure
//! using testcontainers. Only available with the `containers` feature.
//!
//! # Usage
//!
//! Add to your crate's `Cargo.toml`:
//!
//! ```toml
//! [dev-dependencies]
//! test-utils = { path = "../test-utils", features = ["containers"] }
//! ```
//!
//! Then in your integration tests:
//!
//! ```ignore
//! use test_utils::containers::TestInfrastructure;
//!
//! #[tokio::test]
//! async fn test_with_real_services() {
//!     let infra = TestInfrastructure::start().await;
//!
//!     // Use infra.postgres_url(), infra.redis_url(), infra.minio_url()
//!     // to connect to the containers
//! }
//! ```

use std::time::Duration;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};

/// Test infrastructure with PostgreSQL, Redis, and MinIO containers.
///
/// All containers are automatically cleaned up when this struct is dropped.
pub struct TestInfrastructure {
    /// PostgreSQL container - held to keep it running, accessed via `postgres_url()`.
    #[allow(dead_code)]
    postgres: ContainerAsync<GenericImage>,
    /// Redis container - held to keep it running, accessed via `redis_url()`.
    #[allow(dead_code)]
    redis: ContainerAsync<GenericImage>,
    /// MinIO container - held to keep it running, accessed via `minio_url()`.
    #[allow(dead_code)]
    minio: ContainerAsync<GenericImage>,
    postgres_port: u16,
    redis_port: u16,
    minio_port: u16,
}

impl TestInfrastructure {
    /// Start all infrastructure containers.
    ///
    /// This will start PostgreSQL (with PostGIS), Redis, and MinIO containers
    /// with health checks to ensure they're ready before returning.
    pub async fn start() -> Self {
        // Start containers in parallel for faster startup
        let (postgres, redis, minio) = tokio::join!(
            Self::start_postgres(),
            Self::start_redis(),
            Self::start_minio()
        );

        let postgres_port = postgres.get_host_port_ipv4(5432).await.unwrap();
        let redis_port = redis.get_host_port_ipv4(6379).await.unwrap();
        let minio_port = minio.get_host_port_ipv4(9000).await.unwrap();

        // Wait a bit for services to be fully ready
        tokio::time::sleep(Duration::from_millis(500)).await;

        Self {
            postgres,
            redis,
            minio,
            postgres_port,
            redis_port,
            minio_port,
        }
    }

    async fn start_postgres() -> ContainerAsync<GenericImage> {
        GenericImage::new("postgis/postgis", "16-3.4")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_USER", "test")
            .with_env_var("POSTGRES_PASSWORD", "test")
            .with_env_var("POSTGRES_DB", "test")
            .with_startup_timeout(Duration::from_secs(60))
            .start()
            .await
            .expect("Failed to start PostgreSQL container")
    }

    async fn start_redis() -> ContainerAsync<GenericImage> {
        GenericImage::new("redis", "7-bookworm")
            .with_exposed_port(6379.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
            .with_startup_timeout(Duration::from_secs(30))
            .start()
            .await
            .expect("Failed to start Redis container")
    }

    async fn start_minio() -> ContainerAsync<GenericImage> {
        GenericImage::new("minio/minio", "latest")
            .with_exposed_port(9000.tcp())
            .with_env_var("MINIO_ROOT_USER", "minioadmin")
            .with_env_var("MINIO_ROOT_PASSWORD", "minioadmin")
            .with_cmd(vec!["server", "/data"])
            .with_startup_timeout(Duration::from_secs(30))
            .start()
            .await
            .expect("Failed to start MinIO container")
    }

    /// Get the PostgreSQL connection URL.
    pub fn postgres_url(&self) -> String {
        format!(
            "postgresql://test:test@127.0.0.1:{}/test",
            self.postgres_port
        )
    }

    /// Get the Redis connection URL.
    pub fn redis_url(&self) -> String {
        format!("redis://127.0.0.1:{}", self.redis_port)
    }

    /// Get the MinIO endpoint URL.
    pub fn minio_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.minio_port)
    }

    /// Get the MinIO port.
    pub fn minio_port(&self) -> u16 {
        self.minio_port
    }

    /// Get the PostgreSQL port.
    pub fn postgres_port(&self) -> u16 {
        self.postgres_port
    }

    /// Get the Redis port.
    pub fn redis_port(&self) -> u16 {
        self.redis_port
    }

    /// Create a bucket in MinIO using the mc client inside the container.
    ///
    /// This uses `docker exec` to run the MinIO client inside the container.
    pub async fn create_minio_bucket(&self, bucket: &str) -> Result<(), String> {
        use std::process::Command;

        let container_id = self.minio.id().chars().take(12).collect::<String>();

        // Configure mc alias inside the container
        let configure_result = Command::new("docker")
            .args([
                "exec",
                &container_id,
                "mc",
                "alias",
                "set",
                "local",
                "http://localhost:9000",
                "minioadmin",
                "minioadmin",
            ])
            .output();

        if let Err(e) = configure_result {
            return Err(format!("Failed to configure mc: {}", e));
        }

        // Create the bucket
        let create_result = Command::new("docker")
            .args([
                "exec",
                &container_id,
                "mc",
                "mb",
                "--ignore-existing",
                &format!("local/{}", bucket),
            ])
            .output();

        match create_result {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    // Ignore "already exists" errors
                    if !stderr.contains("already") && !stderr.is_empty() {
                        return Err(format!("Failed to create bucket: {}", stderr));
                    }
                }
                Ok(())
            }
            Err(e) => Err(format!("Failed to run mc mb: {}", e)),
        }
    }
}

/// Configuration for connecting to test infrastructure.
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub database_url: String,
    pub redis_url: String,
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
}

impl TestConfig {
    /// Create a test configuration from infrastructure.
    pub fn from_infrastructure(infra: &TestInfrastructure) -> Self {
        Self {
            database_url: infra.postgres_url(),
            redis_url: infra.redis_url(),
            s3_endpoint: infra.minio_url(),
            s3_bucket: "test-bucket".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
        }
    }

    /// Set environment variables for services that read from env.
    pub fn set_env_vars(&self) {
        std::env::set_var("DATABASE_URL", &self.database_url);
        std::env::set_var("REDIS_URL", &self.redis_url);
        std::env::set_var("S3_ENDPOINT", &self.s3_endpoint);
        std::env::set_var("S3_BUCKET", &self.s3_bucket);
        std::env::set_var("S3_ACCESS_KEY", &self.s3_access_key);
        std::env::set_var("S3_SECRET_KEY", &self.s3_secret_key);
        std::env::set_var("S3_REGION", &self.s3_region);
        std::env::set_var("S3_ALLOW_HTTP", "true");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_infrastructure_starts() {
        let infra = TestInfrastructure::start().await;

        // Verify URLs are generated correctly
        assert!(infra.postgres_url().contains("postgresql://"));
        assert!(infra.redis_url().contains("redis://"));
        assert!(infra.minio_url().contains("http://"));

        // Verify ports are non-zero
        assert!(infra.postgres_port() > 0);
        assert!(infra.redis_port() > 0);
        assert!(infra.minio_port() > 0);
    }

    #[tokio::test]
    #[ignore] // Requires Docker
    async fn test_config_from_infrastructure() {
        let infra = TestInfrastructure::start().await;
        let config = TestConfig::from_infrastructure(&infra);

        assert!(config.database_url.contains("test:test"));
        assert_eq!(config.s3_bucket, "test-bucket");
        assert_eq!(config.s3_access_key, "minioadmin");
    }
}
