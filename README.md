# Atlas Tools
>[!WARNING]
> This is a work in progress and is not yet ready for use.

## Overview

Atlas Tools is a Rust-based package management system designed for building and distributing software packages. It provides a comprehensive framework for package building, dependency resolution, and package distribution.

## Key Features

1. **Package Building**: Supports building packages from source with sandboxed environments
2. **Dependency Resolution**: Advanced dependency resolution with artifact store lookup
3. **Package Distribution**: Efficient package distribution and versioning
4. **Recipe Format**: Standardized recipe format for package definitions
5. **Checksum Verification**: Built-in checksum verification for package integrity

## Architecture

The system consists of several core components:

1. **Sisyphus Core**: The main package building and management engine
2. **Fetcher Module**: Handles package source fetching (HTTP, Git)
3. **Checksum Module**: Provides cryptographic verification of package contents
4. **Dependency Resolver**: Manages package dependencies and artifact store lookup
5. **Recipe Parser**: Processes package definition files

## Package Format

Atlas Tools uses a standardized recipe format with these key fields:

- `schema`: Version of the recipe format
- `name`: Package name
- `version`: Upstream package version
- `release`: Atlas packaging revision
- `architecture`: Target CPU architecture
- `source`: Package source with URL and checksum
- `makedeps`: Build-time dependencies
- `deps`: Runtime dependencies

## Getting Started

### Prerequisites

- Rust toolchain (latest stable version)
- Git

### Installation

```bash
cargo build --release
```

### Basic Usage

1. Create a recipe file in the standardized format
2. Run the package builder:

```bash
cargo run --bin sisyphus -- build <recipe-file>
```

## Development

The project follows these development practices:

1. **Atomic Commits**: Small, focused commits with clear messages
2. **Feature Branching**: Development happens on feature branches
3. **Code Reviews**: All changes require review before merging

## Contributing

Contributions are welcome! Please follow these guidelines:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request with a clear description

## License

This project is licensed under the GNU General Public License v3.0 - see the LICENSE file for details.
