# AGENTS.md

## Build, Lint, and Test Commands

**Build all crates:**
```bash
cargo build
```

**Run all tests:**
```bash
cargo test
```

**Run tests for a specific crate:**
```bash
cargo test --package wms-common
cargo test --package grib2-parser
# etc. for any crate in crates/ or services/
```

**Run a single test by name:**
```bash
cargo test test_name -- --exact
```

**Run tests with output:**
```bash
cargo test -- --nocapture
```

**Build a specific service:**
```bash
cargo build --package wms-api
```

**Check code without building:**
```bash
cargo check
```

**Format code:**
```bash
cargo fmt
```

**Run clippy linter:**
```bash
cargo clippy -- -D warnings
```

## Code Style Guidelines

### Imports
- Organize imports in three groups (separated by blank lines): std library, external crates, local modules
- Use `use` statements for public APIs, not glob imports in library code
- Prefer aliasing with `as` for disambiguation

### Formatting
- Use 4-space indentation (configured by Cargo default)
- Run `cargo fmt` before committing
- Wrap lines at reasonable lengths for readability

### Type System
- Prefer explicit type annotations in function signatures
- Use type aliases for complex types (e.g., `WmsResult<T> = Result<T, WmsError>`)
- Leverage Rust's type system to prevent errors at compile time

### Naming Conventions
- **Crate names:** lowercase with hyphens (e.g., `wms-common`, `grib2-parser`)
- **Module/function names:** snake_case
- **Type/trait names:** PascalCase
- **Constants:** SCREAMING_SNAKE_CASE

### Error Handling
- Always return `Result<T, E>` from fallible operations
- Use custom error types with `thiserror` derive macro for domain-specific errors
- Implement `From` impls for automatic error conversion
- Map WMS errors to OGC exception codes and HTTP status codes in `WmsError`
- Use `WmsResult<T>` alias for cleaner function signatures

### Documentation
- Add doc comments (`///`) for all public items
- Use module-level doc comments (`//!`) explaining module purpose
- Include examples in doc comments where helpful

### Testing
- Write unit tests in the same file as code using `#[cfg(test)]` modules
- Name test functions descriptively (e.g., `test_invalid_bbox_returns_error`)
- Use `tokio::test` for async test functions
- Test both success and error paths

### Async/Concurrency
- Use `tokio` as the async runtime (configured workspace-wide)
- Mark async functions with `#[tokio::main]` or `#[tokio::test]`
- Use `Arc` for shared state across async tasks
- Prefer `tracing` over println! for logging

### Dependencies
- Keep workspace dependencies synchronized in `Cargo.toml [workspace.dependencies]`
- Use workspace inheritance for common crates (tokio, serde, axum, etc.)
- Avoid duplicate dependency versions across the workspace
