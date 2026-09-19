#!/usr/bin/env bash
# Install the nitrum-fn CLI from GitHub Releases (assets built by .github/workflows/release.yml).
set -euo pipefail

NITRUM_FN_REPO="${NITRUM_FN_REPO:-matzapata/nitrum-fn}"
NITRUM_FN_VERSION="${NITRUM_FN_VERSION:-${1:-latest}}"
NITRUM_FN_INSTALL_DIR="${NITRUM_FN_INSTALL_DIR:-$HOME/.local/bin}"

# Asset basenames match release.yml: nitrum-fn-${{ matrix.suffix }}${{ matrix.ext }}
detect_asset_name() {
  local os arch suffix ext=""
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
  Linux)
    case "$arch" in
    x86_64)
      suffix="linux-x86_64"
      ;;
    aarch64 | arm64)
      echo "unsupported: Linux aarch64 has no prebuilt binary in this release; build with: cargo install --git https://github.com/${NITRUM_FN_REPO} --locked --bin nitrum-fn" >&2
      return 1
      ;;
    *)
      echo "unsupported: Linux ARCH=$arch — see https://github.com/${NITRUM_FN_REPO}/releases" >&2
      return 1
      ;;
    esac
    ;;
  Darwin)
    case "$arch" in
    arm64)
      suffix="darwin-aarch64"
      ;;
    x86_64)
      echo "unsupported: macOS Intel (x86_64) has no prebuilt binary; use Apple Silicon or build from source: cargo install --git https://github.com/${NITRUM_FN_REPO} --locked --bin nitrum-fn" >&2
      return 1
      ;;
    *)
      echo "unsupported: Darwin ARCH=$arch — see https://github.com/${NITRUM_FN_REPO}/releases" >&2
      return 1
      ;;
    esac
    ;;
  MINGW* | MSYS* | CYGWIN*)
    case "$arch" in
    x86_64)
      suffix="windows-x86_64"
      ext=".exe"
      ;;
    *)
      echo "unsupported: Windows ARCH=$arch — see https://github.com/${NITRUM_FN_REPO}/releases" >&2
      return 1
      ;;
    esac
    ;;
  *)
    echo "unsupported: OS=$os ARCH=$arch — see https://github.com/${NITRUM_FN_REPO}/releases" >&2
    return 1
    ;;
  esac

  echo "nitrum-fn-${suffix}${ext}"
}

asset_name="$(detect_asset_name)" || exit 1
is_windows=0
[[ "$asset_name" == *.exe ]] && is_windows=1

if [[ "$NITRUM_FN_VERSION" == "latest" ]]; then
  url="https://github.com/${NITRUM_FN_REPO}/releases/latest/download/${asset_name}"
else
  ver="$NITRUM_FN_VERSION"
  [[ "$ver" == v* ]] || ver="v${ver}"
  url="https://github.com/${NITRUM_FN_REPO}/releases/download/${ver}/${asset_name}"
fi

mkdir -p "$NITRUM_FN_INSTALL_DIR"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

echo "Downloading ${url}"
curl -fSL --retry 3 --retry-delay 1 -o "$tmp" "$url"

dest="${NITRUM_FN_INSTALL_DIR}/nitrum-fn"
[[ "$is_windows" -eq 1 ]] && dest="${dest}.exe"

mv "$tmp" "$dest"
trap - EXIT
[[ "$is_windows" -eq 0 ]] && chmod +x "$dest"

echo "Installed: $dest"

case ":${PATH:-}:" in
*":${NITRUM_FN_INSTALL_DIR}:"*) ;;
*)
  echo "Add ${NITRUM_FN_INSTALL_DIR} to your PATH, for example:" >&2
  echo "  export PATH=\"${NITRUM_FN_INSTALL_DIR}:\$PATH\"" >&2
  ;;
esac

if [[ "$is_windows" -eq 0 ]]; then
  "$dest" --help >/dev/null 2>&1 && echo "Run: nitrum-fn --help"
fi
