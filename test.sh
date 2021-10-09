#!/bin/bash
set -eu

export RUST_BACKTRACE=1
export RUSTFLAGS="-D dead_code -D unused-variables -D unused"

for feature in "" json charset cookies socks-proxy ; do
  if ! cargo test --no-default-features --features "${feature}" ; then
    echo Command failed: cargo test --no-default-features --features \"${tls} ${feature}\"
    exit 1
  fi
done
