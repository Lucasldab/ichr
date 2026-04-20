//! Cancellation support. We re-export `tokio_util::sync::CancellationToken`
//! so downstream code has a single canonical import path (`crate::tasks::CancellationToken`)
//! even if we swap the impl later.

pub use tokio_util::sync::CancellationToken;
