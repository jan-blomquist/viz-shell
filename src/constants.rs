/// The repository configuration file, at the repository root.
pub const REPO_CONFIG_FILE: &str = "vz.yml";

/// Our own variable rather than `RUST_LOG`, which the repository's
/// tools may set for themselves.
pub const LOG_ENV: &str = "VZ_LOG";

/// Warnings from everything, plus vz's own progress lines.
pub const DEFAULT_LOG_FILTER: &str = "warn,vz=info";
