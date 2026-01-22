//! Resource limits for shell execution

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Resource limits for shell execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum CPU time in milliseconds
    pub max_cpu_ms: u64,
    /// Maximum memory in bytes
    pub max_memory_bytes: u64,
    /// Maximum output (stdout + stderr) in bytes
    pub max_output_bytes: u64,
    /// Wall-clock timeout
    #[serde(with = "duration_ms")]
    pub timeout: Duration,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_cpu_ms: 5000,                   // 5 seconds CPU
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_output_bytes: 1024 * 1024,      // 1 MB output
            timeout: Duration::from_secs(30),   // 30 second wall clock
        }
    }
}

/// Helper for serializing Duration as milliseconds
mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ms = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Buffer that limits how much data can be written
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LimitedBuffer {
    buffer: Vec<u8>,
    limit: usize,
    truncated: bool,
}

#[allow(dead_code)]
impl LimitedBuffer {
    pub fn new(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            limit,
            truncated: false,
        }
    }

    pub fn write(&mut self, data: &[u8]) -> usize {
        let remaining = self.limit.saturating_sub(self.buffer.len());
        if remaining == 0 {
            self.truncated = true;
            return data.len(); // Pretend we wrote it
        }

        let to_write = data.len().min(remaining);
        self.buffer.extend_from_slice(&data[..to_write]);

        if to_write < data.len() {
            self.truncated = true;
            self.buffer
                .extend_from_slice(b"\n... [output truncated] ...\n");
        }

        data.len()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    pub fn was_truncated(&self) -> bool {
        self.truncated
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }
}

impl std::io::Write for LimitedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(LimitedBuffer::write(self, buf))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::write_literal)]
mod tests {
    use super::*;
    use std::io::Write as _;

    // ==================== ResourceLimits Tests ====================

    #[test]
    fn test_default_limits() {
        let limits = ResourceLimits::default();

        assert_eq!(limits.max_cpu_ms, 5000);
        assert_eq!(limits.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.max_output_bytes, 1024 * 1024);
        assert_eq!(limits.timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_limits_serialization() {
        let limits = ResourceLimits {
            max_cpu_ms: 10000,
            max_memory_bytes: 128 * 1024 * 1024,
            max_output_bytes: 2 * 1024 * 1024,
            timeout: Duration::from_secs(60),
        };

        let json = serde_json::to_string(&limits).unwrap();
        let deserialized: ResourceLimits = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.max_cpu_ms, 10000);
        assert_eq!(deserialized.max_memory_bytes, 128 * 1024 * 1024);
        assert_eq!(deserialized.max_output_bytes, 2 * 1024 * 1024);
        assert_eq!(deserialized.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_limits_serialization_format() {
        let limits = ResourceLimits {
            max_cpu_ms: 1000,
            max_memory_bytes: 1024,
            max_output_bytes: 512,
            timeout: Duration::from_millis(5000),
        };

        let json = serde_json::to_string(&limits).unwrap();

        // Timeout should be serialized as milliseconds
        assert!(json.contains("\"timeout\":5000"));
    }

    // ==================== LimitedBuffer Tests ====================

    #[test]
    fn test_limited_buffer_under_limit() {
        let mut buffer = LimitedBuffer::new(100);

        let written = buffer.write(b"hello world");
        assert_eq!(written, 11);
        assert!(!buffer.was_truncated());
        assert_eq!(buffer.as_bytes(), b"hello world");
    }

    #[test]
    fn test_limited_buffer_at_limit() {
        let mut buffer = LimitedBuffer::new(5);

        let written = buffer.write(b"hello");
        assert_eq!(written, 5);
        assert!(!buffer.was_truncated());
        assert_eq!(buffer.as_bytes(), b"hello");
    }

    #[test]
    fn test_limited_buffer_over_limit_truncates() {
        let mut buffer = LimitedBuffer::new(5);

        let written = buffer.write(b"hello world");
        assert_eq!(written, 11); // Reports full length written (pretends)
        assert!(buffer.was_truncated());

        // Should contain partial data plus truncation message
        let content = buffer.as_bytes();
        assert!(content.starts_with(b"hello"));
        assert!(content.len() > 5); // Contains truncation message
    }

    #[test]
    fn test_limited_buffer_truncation_message() {
        let mut buffer = LimitedBuffer::new(10);

        buffer.write(b"hello world longer text");

        let content = String::from_utf8_lossy(buffer.as_bytes());
        assert!(content.contains("truncated"));
    }

    #[test]
    fn test_limited_buffer_multiple_writes() {
        let mut buffer = LimitedBuffer::new(20);

        buffer.write(b"hello ");
        assert!(!buffer.was_truncated());

        buffer.write(b"world ");
        assert!(!buffer.was_truncated());

        buffer.write(b"this is a long message");
        assert!(buffer.was_truncated());
    }

    #[test]
    fn test_limited_buffer_write_after_truncation() {
        let mut buffer = LimitedBuffer::new(5);

        buffer.write(b"hello world");
        assert!(buffer.was_truncated());

        // Writing more after truncation should still "succeed"
        let written = buffer.write(b"more data");
        assert_eq!(written, 9);
        assert!(buffer.was_truncated());
    }

    #[test]
    fn test_limited_buffer_into_bytes() {
        let mut buffer = LimitedBuffer::new(100);
        buffer.write(b"test data");

        let bytes = buffer.into_bytes();
        assert_eq!(bytes, b"test data");
    }

    #[test]
    fn test_limited_buffer_empty() {
        let buffer = LimitedBuffer::new(100);
        assert!(!buffer.was_truncated());
        assert!(buffer.as_bytes().is_empty());
    }

    #[test]
    fn test_limited_buffer_zero_limit() {
        let mut buffer = LimitedBuffer::new(0);

        let written = buffer.write(b"hello");
        assert_eq!(written, 5);
        assert!(buffer.was_truncated());
    }

    #[test]
    fn test_limited_buffer_io_write_trait() {
        let mut buffer = LimitedBuffer::new(100);

        // Use std::io::Write trait
        writeln!(buffer, "hello {}", "world").unwrap();
        buffer.flush().unwrap();

        assert!(buffer.as_bytes().starts_with(b"hello world\n"));
    }

    #[test]
    fn test_limited_buffer_exact_boundary() {
        // Test writing exactly at the boundary
        let mut buffer = LimitedBuffer::new(10);

        buffer.write(b"12345");
        assert!(!buffer.was_truncated());
        assert_eq!(buffer.as_bytes().len(), 5);

        buffer.write(b"67890");
        assert!(!buffer.was_truncated());
        assert_eq!(buffer.as_bytes().len(), 10);

        // Next write should trigger truncation
        buffer.write(b"x");
        assert!(buffer.was_truncated());
    }
}
