#!/bin/sh
# Builds the two Linux forms of sitegen, from this directory (the addon with the profile in pgo/: profile-guided
# optimization):
#   ../sitegen-linux-x64.node  the Node addon (x86-64 Linux, glibc): run on Debian 12 with Rust and gcc, as the
#                              grader's node:22 image is (for example: docker run -v "$PWD/..":/repo node:22 ...,
#                              apt-get install gcc, rustup)
#   ../sitegen-linux-x64       the static program (musl, linked by rust-lld; .cargo/config.toml), from any host; it
#                              runs only where the addon does not load, and is built without the profile
# pgo/merged.profdata is llvm-profdata's merge of the program's profiles over the kit's practice set and sites drawn
# with its generator (an instrumented build: -C profile-generate).
set -e
PGO="-C target-cpu=x86-64-v2 -C profile-use=$(pwd)/pgo/merged.profdata"
case "$1" in
  node)
    RUSTFLAGS="$PGO" cargo build --release --lib --target x86_64-unknown-linux-gnu
    cp "${CARGO_TARGET_DIR:-target}/x86_64-unknown-linux-gnu/release/libsitegen.so" ../sitegen-linux-x64.node ;;
  program)
    cargo build --release --bin sitegen --target x86_64-unknown-linux-musl
    cp "${CARGO_TARGET_DIR:-target}/x86_64-unknown-linux-musl/release/sitegen" ../sitegen-linux-x64 ;;
  *) echo "usage: build-linux.sh node|program"; exit 2 ;;
esac
