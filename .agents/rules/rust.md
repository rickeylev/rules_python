---
trigger: glob
description: Rust coding, dependency vetting, and rules_rust crate conventions
globs: "*.rs,Cargo.toml,Cargo.lock,Cargo.Bazel.lock,MODULE.bazel"
---

# Rust Conventions

## Standard Libraries Over Custom Subsystems
* Prefer `std` and established ecosystem crates over implementing non-trivial
  subsystems from scratch (e.g., regular expression engines, ZIP/archive
  processing, compression codecs, cryptographic hashes, or format parsers).

## Dependency Vetting & Versioning
When adding or updating a Rust crate dependency:
* **Verify legitimacy**: Confirm the crate's maintainers and source repository
  on `crates.io` (prefer established organizations such as `rust-lang`,
  `RustCrypto`, or `zip-rs`), check corporate `cargo-vet` audit logs (Chromium
  or Mozilla `audits.toml`), and check `rustsec.org` for advisories.
* **Use the latest version**: Query `index.crates.io` or `crates.io` and pin
  the latest non-yanked stable release. Document any version ceiling required
  by MSRV or cross-compilation platform constraints in a comment next to the
  dependency spec.
* **Prune features & C dependencies**: Set `default_features = False` and
  enable only required `features` when defaults pull in unused crates or C
  `-sys` libraries, keeping cross-compiled and static `musl` builds free of C
  toolchain requirements.
* **Regenerate lockfiles**: Update `Cargo.lock` and `Cargo.Bazel.lock` with
  `CARGO_BAZEL_REPIN=1`; never hand-edit generated lockfiles.

## Crate Structure & Formatting
* Place core logic in library crates (`src/lib.rs`) with thin CLI entry points
  so unit tests call library functions directly.
* Format Rust sources with `bazel run @rules_rust//:rustfmt` and verify with
  `--aspects=@rules_rust//rust:defs.bzl%rustfmt_aspect`
  `--output_groups=+rustfmt_checks`.
