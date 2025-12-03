# Contributing

Thank you for your interest in contributing to Weather WMS! This guide will help you get started.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork**:
   ```bash
   git clone https://github.com/YOUR_USERNAME/JoeGCServices.git
   cd JoeGCServices
   ```
3. **Add upstream remote**:
   ```bash
   git remote add upstream https://github.com/JoegottabeGitenme/JoeGCServices.git
   ```
4. **Create a branch**:
   ```bash
   git checkout -b feature/my-feature
   ```

## Development Setup

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install dev tools
cargo install cargo-watch cargo-edit cargo-audit

# Build and test
cargo build
cargo test --workspace
```

## Making Changes

### Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix warnings
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Add documentation comments for public APIs
- Write tests for new functionality

### Commit Messages

Use conventional commits format:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types**:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `perf`: Performance improvements
- `test`: Adding/updating tests
- `chore`: Maintenance tasks

**Examples**:
```
feat(renderer): add wind barb visualization

Implements SVG-based wind barb rendering for vector wind data.
Supports standard WMO wind barb conventions with configurable
spacing and scaling.

Closes #123
```

```
fix(wms-api): correct CRS axis order for EPSG:4326

WMS 1.3.0 requires lat,lon order for EPSG:4326, not lon,lat.
This fixes incorrect tile rendering for WMS 1.3.0 clients.

Fixes #456
```

### Testing

- Write unit tests in `#[cfg(test)] mod tests`
- Add integration tests in `tests/` directory
- Ensure all tests pass: `cargo test --workspace`
- Add benchmarks for performance-critical code

### Documentation

- Add inline documentation with `///` comments
- Update relevant .md files in `docs/src/`
- Include examples in documentation:
  ```rust
  /// Parses a GRIB2 file.
  ///
  /// # Example
  ///
  /// ```
  /// use grib2_parser::parse_file;
  /// let messages = parse_file("data.grib2")?;
  /// ```
  pub fn parse_file(path: &str) -> Result<Vec<Message>> {
      // ...
  }
  ```

## Pull Request Process

1. **Update your branch**:
   ```bash
   git fetch upstream
   git rebase upstream/main
   ```

2. **Push to your fork**:
   ```bash
   git push origin feature/my-feature
   ```

3. **Create Pull Request** on GitHub:
   - Provide clear title and description
   - Reference related issues
   - Add screenshots/examples if applicable
   - Check the PR template checkboxes

4. **Address review feedback**:
   - Make requested changes
   - Push additional commits
   - Respond to comments

5. **Merge**: Once approved, your PR will be merged!

## Pull Request Template

```markdown
## Description
Brief description of changes.

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Testing
- [ ] Unit tests added/updated
- [ ] Integration tests added/updated
- [ ] Manual testing performed

## Checklist
- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Documentation updated
- [ ] Tests pass locally
- [ ] No new warnings from clippy
```

## Areas for Contribution

### Good First Issues

Look for issues labeled `good first issue`:
- Documentation improvements
- Adding tests
- Fixing typos
- Small bug fixes

### Feature Requests

Check issues labeled `enhancement`:
- New data sources
- Additional visualization styles
- Performance optimizations
- New API features

### Bug Reports

Issues labeled `bug`:
- Reproduce the bug
- Write failing test
- Fix the bug
- Verify test passes

## Code Review

Reviews focus on:
- **Correctness**: Does it work as intended?
- **Testing**: Are changes adequately tested?
- **Performance**: Any performance implications?
- **Maintainability**: Is code readable and well-structured?
- **Documentation**: Are changes documented?

## Communication

- **GitHub Issues**: Bug reports, feature requests
- **GitHub Discussions**: Questions, ideas, general discussion
- **Pull Requests**: Code changes

## License

By contributing, you agree that your contributions will be licensed under the same license as the project.

## Recognition

Contributors are recognized in:
- GitHub contributors page
- CONTRIBUTORS.md (if added)
- Release notes for significant contributions

## Questions?

- Check existing [documentation](https://weather-wms.io)
- Search [closed issues](https://github.com/JoegottabeGitenme/JoeGCServices/issues?q=is%3Aissue+is%3Aclosed)
- Ask in [GitHub Discussions](https://github.com/JoegottabeGitenme/JoeGCServices/discussions)

## Next Steps

- [Development Guide](./README.md) - Development overview
- [Building](./building.md) - Build from source
- [Testing](./testing.md) - Run tests
- [Benchmarking](./benchmarking.md) - Performance testing
