#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SOURCE="$PROJECT_ROOT/src/a26-shell"
IMAGE="$PROJECT_ROOT/images/a26-shell-0.1.0"
TARGET=aarch64-unknown-linux-musl

rustup target add "$TARGET" >/dev/null
# Cargo only discovers .cargo/config.toml by walking up from its current
# directory (not from --manifest-path), so build from the source directory to
# select the musl cross-linker pinned there.
(
    cd "$SOURCE"
    cargo fmt --check
    cargo test --locked
    cargo build --locked --release --target "$TARGET"
)

mkdir -p "$IMAGE/bin" "$IMAGE/source"
install -m0755 "$SOURCE/target/$TARGET/release/a26-shell" "$IMAGE/bin/a26-shell"
install -m0755 "$SOURCE/target/$TARGET/release/a26-shellctl" "$IMAGE/bin/a26-shellctl"
install -m0755 "$SOURCE/target/$TARGET/release/a26-shellshot" "$IMAGE/bin/a26-shellshot"
tar --exclude=target -czf "$IMAGE/source/a26-shell-0.1.0.tar.gz" \
    -C "$PROJECT_ROOT/src" a26-shell

cat >"$IMAGE/MANIFEST.txt" <<EOF
name=a26-shell
version=0.1.0
target=$TARGET
source=src/a26-shell
display=:0
resolution=1080x2340
EOF

(cd "$IMAGE" && sha256sum MANIFEST.txt bin/a26-shell bin/a26-shellctl bin/a26-shellshot \
    source/a26-shell-0.1.0.tar.gz > SHA256SUMS)
(cd "$IMAGE" && sha256sum -c SHA256SUMS)
