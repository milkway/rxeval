#!/usr/bin/env sh
# Gather the Rust sources CRAN will build without a network.
#
#   sh tools/vendor.sh
#
# CRAN's machines have no internet during the build, so every crate the
# package compiles has to travel inside the tarball. `cargo vendor` copies
# them, and the config file it prints is what tells cargo to use the copies
# instead of reaching out — without that file the extracted sources are
# ignored and the build fails looking for the network.
#
# Neither output is committed: they are a build product of Cargo.lock, and a
# 1.6 MB binary that changes with every dependency bump does not belong in
# a repository's history. Run this before `R CMD build`.
set -eu
cd "$(dirname "$0")/../src/rust"
rm -rf vendor vendor.tar.xz
cargo vendor --versioned-dirs vendor > vendor-config.toml
tar -cJf vendor.tar.xz vendor
rm -rf vendor
printf 'vendor.tar.xz: %s\n' "$(du -h vendor.tar.xz | cut -f1)"
