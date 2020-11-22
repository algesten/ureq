#!/bin/bash
set -eu

export RUST_BACKTRACE=1
export RUSTFLAGS="-D dead_code -D unused-variables -D unused"

for tls in "" tls ; do
  for feature in "" json charset cookies socks-proxy ; do
    if ! cargo test --no-default-features --features "${tls} ${feature}" ; then
      echo Command failed: cargo test \"${what}\" --no-default-features --features \"${tls} ${feature}\"
      exit 1
    fi
  done
done
