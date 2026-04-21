# hopr-api

[![Crates.io](https://img.shields.io/crates/v/hopr-api)](https://crates.io/crates/hopr-api)
[![docs.rs](https://img.shields.io/docsrs/hopr-api)](https://docs.rs/hopr-api)
[![CI](https://github.com/hoprnet/hopr-api/actions/workflows/merge.yaml/badge.svg?branch=main)](https://github.com/hoprnet/hopr-api/actions/workflows/merge.yaml)
[![Security](https://github.com/hoprnet/hopr-api/actions/workflows/checks-zizmor.yaml/badge.svg)](https://github.com/hoprnet/hopr-api/actions/workflows/checks-zizmor.yaml)
[![License](https://img.shields.io/crates/l/hopr-api)](LICENSE)
[![MSRV](https://img.shields.io/crates/msrv/hopr-api)](https://github.com/hoprnet/hopr-api)
[![Crates.io Downloads](https://img.shields.io/crates/d/hopr-api)](https://crates.io/crates/hopr-api)

Common high-level API traits for the [HOPR protocol](https://hoprnet.org/).

This crate defines the public API surface as **traits only** — no concrete implementations.
Implementations live in the main [hoprnet](https://github.com/hoprnet/hoprnet) repository.

## Modules

All modules are feature-gated:

| Feature   | Module    | Description                                            |
| --------- | --------- | ------------------------------------------------------ |
| `chain`   | `chain`   | On-chain operation APIs (accounts, channels, tickets…) |
| `ct`      | `ct`      | Cover traffic and probing API traits                   |
| `db`      | `db`      | Node database operation traits                         |
| `graph`   | `graph`   | Network graph topology, QoS, routing costs             |
| `network` | `network` | Network state, peer observations, connectivity         |
| `node`    | `node`    | High-level HOPR node API traits and state machine      |
| `full`    | _all_     | Enables all of the above + `serde`                     |

## Usage

```toml
[dependencies]
hopr-api = { git = "https://github.com/hoprnet/hopr-api", features = ["full"] }
```

## Development

Requires [Nix](https://nixos.org/) with flakes enabled.

```bash
# Enter development shell
nix develop

# Build
cargo build

# Test
cargo test --lib

# Lint
cargo clippy

# Format
nix fmt
```

## License

[GPL-3.0-only](LICENSE)
