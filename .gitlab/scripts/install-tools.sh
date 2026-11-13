#!/bin/sh
# Installs pinned tools into <dir> after verifying the SHA-256 checksum of
# each download; there is no fallback to another source or to a build. The
# executables go to <dir>/bin, zig's distribution beside them. Two hosts run
# the pipeline's jobs: x86_64 Linux, the group's runners, which run every
# job but the macOS builds and need every tool but rustup, and arm64
# macOS, the project's own runner, which builds the macOS targets and needs
# only rustup and cargo-about; a tool not pinned for the host is refused. A
# new version of a tool needs both its version and its checksums below; a
# checksum is the download's SHA-256, as its release lists it and as a
# download shows it.
#
# usage: .gitlab/scripts/install-tools.sh <dir> <tool>...
#   tools: zig, cargo-zigbuild, cargo-about, cargo-deny, release-plz, cosign,
#   jq, shellcheck (x86_64 Linux); rustup, cargo-about (arm64 macOS)
set -eu

dir=${1:?usage: install-tools.sh <dir> <tool>...}
shift
host="$(uname -s)-$(uname -m)"
case "$host" in
  Linux-x86_64) sha256() { sha256sum "$@"; } ;;
  Darwin-arm64) sha256() { shasum -a 256 "$@"; } ;;
  *) echo "the pinned tools are for x86_64 Linux and arm64 macOS, not $host" >&2; exit 1 ;;
esac
mkdir -p "$dir/bin"
dir=$(cd "$dir" && pwd -P)

unpinned() {
  echo "$tool is not pinned for $host" >&2
  exit 1
}

fetch() { # <url> <sha256> <file>
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --output "$3" "$1"
  echo "$2  $3" | sha256 --check --strict --quiet -
}

# <url> <sha256> <path of the executable in the archive>
unpack() {
  archive="$dir/download"
  fetch "$1" "$2" "$archive"
  tar -xf "$archive" -C "$dir/bin" --strip-components="$(printf '%s' "$3" | tr -cd / | wc -c)" "$3"
  rm "$archive"
}

for tool in "$@"; do
  case "$tool" in
    zig)
      [ "$host" = Linux-x86_64 ] || unpinned
      # cargo-zigbuild links the Linux release binaries with zig; xtask's
      # notices name the zig distribution, whose musl, libc and compiler_rt
      # it links.
      version=0.17.0
      name="zig-x86_64-linux-$version"
      fetch "https://ziglang.org/download/$version/$name.tar.xz" \
        1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026 "$dir/download"
      rm -rf "${dir:?}/$name"
      tar -xJf "$dir/download" -C "$dir"
      rm "$dir/download"
      ln -sf "../$name/zig" "$dir/bin/zig"
      ;;
    cargo-zigbuild)
      [ "$host" = Linux-x86_64 ] || unpinned
      version=0.23.4
      unpack "https://github.com/rust-cross/cargo-zigbuild/releases/download/v$version/cargo-zigbuild-x86_64-unknown-linux-musl.tar.xz" \
        9e3cf73485edbd45905c8aadbc0fdf869c7ddc3848f0c898229f2680db52e44b \
        cargo-zigbuild-x86_64-unknown-linux-musl/cargo-zigbuild
      ;;
    cargo-about)
      # xtask/src/main.rs refuses to generate the notices with another release.
      version=0.9.2
      case "$host" in
        Linux-x86_64)
          triple=x86_64-unknown-linux-musl
          checksum=9099a59e820c38a68b9d65f300662a567d56562f9a10f6aa4c7e86c17c2566af
          ;;
        Darwin-arm64)
          triple=aarch64-apple-darwin
          checksum=ae72f0df0c399a1e96336f696fa55b1b28679fd725632eba8cf8e4568467cc3e
          ;;
      esac
      unpack "https://github.com/EmbarkStudios/cargo-about/releases/download/$version/cargo-about-$version-$triple.tar.gz" \
        "$checksum" "cargo-about-$version-$triple/cargo-about"
      ;;
    cargo-deny)
      [ "$host" = Linux-x86_64 ] || unpinned
      version=0.20.2
      unpack "https://github.com/EmbarkStudios/cargo-deny/releases/download/$version/cargo-deny-$version-x86_64-unknown-linux-musl.tar.gz" \
        9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f \
        "cargo-deny-$version-x86_64-unknown-linux-musl/cargo-deny"
      ;;
    release-plz)
      [ "$host" = Linux-x86_64 ] || unpinned
      # The glibc build, which the release-plz jobs' Debian image runs. In
      # the musl build, the stack guard of the template parser behind the
      # changelog reads the main thread's stack as only its mapped part and
      # refuses every changelog with "stack limit reached".
      version=0.4.2
      unpack "https://github.com/release-plz/release-plz/releases/download/release-plz-v$version/release-plz-x86_64-unknown-linux-gnu.tar.gz" \
        bd7c3adc1a4ed3831e6bfde51b92b168f32ab6d15cd210ed88044e7f858f0cae \
        release-plz
      ;;
    cosign)
      [ "$host" = Linux-x86_64 ] || unpinned
      version=3.1.4
      fetch "https://github.com/sigstore/cosign/releases/download/v$version/cosign-linux-amd64" \
        424e2c97d0b45fd7a6b608a87f090f35633f4c23ece5f092d48bba11aaf4bbfa "$dir/bin/cosign"
      chmod 0755 "$dir/bin/cosign"
      ;;
    jq)
      [ "$host" = Linux-x86_64 ] || unpinned
      version=1.8.2
      fetch "https://github.com/jqlang/jq/releases/download/jq-$version/jq-linux-amd64" \
        b1c22172dd303f3be49e935aa56aa48a8b7a46e0bc838b4997d3bb451495870f "$dir/bin/jq"
      chmod 0755 "$dir/bin/jq"
      ;;
    shellcheck)
      [ "$host" = Linux-x86_64 ] || unpinned
      version=0.11.0
      unpack "https://github.com/koalaman/shellcheck/releases/download/v$version/shellcheck-v$version.linux.x86_64.tar.xz" \
        8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198 \
        "shellcheck-v$version/shellcheck"
      ;;
    rustup)
      # The macOS builds install their Rust toolchain with rustup-init into
      # the job's RUSTUP_HOME and CARGO_HOME; on Linux the image has it.
      [ "$host" = Darwin-arm64 ] || unpinned
      version=1.29.2
      fetch "https://static.rust-lang.org/rustup/archive/$version/aarch64-apple-darwin/rustup-init" \
        f7c7e11eb2e07f006e532a8aaac77b32f4527d305ac386c0c633ca6133d35ca9 "$dir/bin/rustup-init"
      chmod 0755 "$dir/bin/rustup-init"
      ;;
    *)
      echo "unknown tool: $tool" >&2
      exit 1
      ;;
  esac
  echo "installed $tool"
done
