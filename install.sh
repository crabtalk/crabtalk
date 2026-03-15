#!/bin/sh
# Install script for walrus — https://github.com/openwalrus/walrus
#
# Usage:
#   curl -fsSL openwalrus.xyz/install | sh
#   curl -fsSL openwalrus.xyz/install | sh -s -- --yes              # non-interactive
#   curl -fsSL openwalrus.xyz/install | sh -s -- --services         # install all services
#   curl -fsSL openwalrus.xyz/install | sh -s -- --service memory   # install one service
#
# Environment variables:
#   WALRUS_INSTALL_DIR  Override binary installation directory

set -eu

REPO="openwalrus/walrus"
BINARY_NAME="walrus"
CARGO_CRATE="openwalrus"
AUTO_YES=0
INSTALL_SERVICES=0
INSTALL_SERVICE_LIST=""
TMPDIR_PATH=""

# WHS service binaries and their cargo crate names.
SERVICE_BINS="walrus-memory walrus-search walrus-telegram walrus-discord"

# --- Utility functions ---

setup_colors() {
    if [ -t 2 ]; then
        RED='\033[0;31m'
        GREEN='\033[0;32m'
        YELLOW='\033[1;33m'
        RESET='\033[0m'
    else
        RED=''
        GREEN=''
        YELLOW=''
        RESET=''
    fi
}

info() {
    printf "${GREEN}info${RESET}: %s\n" "$*" >&2
}

warn() {
    printf "${YELLOW}warn${RESET}: %s\n" "$*" >&2
}

err() {
    printf "${RED}error${RESET}: %s\n" "$*" >&2
    exit 1
}

confirm() {
    if [ "$AUTO_YES" = "1" ]; then
        return 0
    fi
    if ! [ -e /dev/tty ]; then
        return 1
    fi
    printf "%s [y/N] " "$1" >/dev/tty
    read -r response </dev/tty
    case "$response" in
        [yY]) return 0 ;;
        *) return 1 ;;
    esac
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        err "need '$1' (command not found)"
    fi
}

check_cmd() {
    command -v "$1" >/dev/null 2>&1
}

# Map a service short name to its binary name.
resolve_service_bin() {
    case "$1" in
        memory|walrus-memory)     echo "walrus-memory" ;;
        search|walrus-search)     echo "walrus-search" ;;
        telegram|walrus-telegram) echo "walrus-telegram" ;;
        discord|walrus-discord)   echo "walrus-discord" ;;
        *) err "unknown service: $1. Available: memory, search, telegram, discord" ;;
    esac
}

# Map a binary name to its cargo crate name.
bin_to_crate() {
    case "$1" in
        walrus-memory)   echo "walrus-memory" ;;
        walrus-search)   echo "walrus-search" ;;
        walrus-telegram) echo "walrus-gateway" ;;
        walrus-discord)  echo "walrus-gateway" ;;
    esac
}

# --- Detection functions ---

parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --yes | -y)
                AUTO_YES=1
                ;;
            --services)
                INSTALL_SERVICES=1
                ;;
            --service)
                shift
                if [ $# -eq 0 ]; then
                    err "--service requires a service name"
                fi
                _bin="$(resolve_service_bin "$1")"
                INSTALL_SERVICE_LIST="${INSTALL_SERVICE_LIST} ${_bin}"
                ;;
            --help | -h)
                cat <<'EOF'
Install walrus — composable primitives for agentic workflows in Rust.

Usage:
  install.sh [OPTIONS]

Options:
  -y, --yes              Skip all confirmation prompts (downloads prebuilt binary)
  --services             Install all WHS services
  --service <name>       Install a specific service (memory, search, telegram, discord)
  -h, --help             Show this help message

Environment variables:
  WALRUS_INSTALL_DIR   Override binary installation directory
EOF
                exit 0
                ;;
            *)
                warn "unknown option: $1"
                ;;
        esac
        shift
    done
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin)  OS="macos" ;;
        Linux)   OS="linux" ;;
        MINGW* | MSYS* | CYGWIN*) OS="windows" ;;
        *)       warn "unrecognized OS: $OS" ;;
    esac

    case "$ARCH" in
        arm64 | aarch64) ARCH="arm64" ;;
        x86_64 | amd64)  ARCH="amd64" ;;
        *)               warn "unrecognized architecture: $ARCH" ;;
    esac
}

detect_existing() {
    EXISTING_PATH=""
    if check_cmd "$BINARY_NAME"; then
        EXISTING_PATH="$(command -v "$BINARY_NAME")"
    fi
}

get_latest_version() {
    # Use the redirect from /releases/latest to extract the tag without the
    # JSON API (avoids rate limits on unauthenticated requests).
    REDIRECT_URL="$(curl -fsSL -o /dev/null -w '%{redirect_url}' \
        "https://github.com/${REPO}/releases/latest" 2>/dev/null || true)"

    if [ -n "$REDIRECT_URL" ]; then
        VERSION="$(printf '%s' "$REDIRECT_URL" | sed 's|.*/||')"
    fi

    # Fallback to the JSON API if redirect didn't work.
    if [ -z "${VERSION:-}" ]; then
        VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')"
    fi

    if [ -z "${VERSION:-}" ]; then
        err "could not determine latest version from GitHub"
    fi

    info "latest version: ${VERSION}"
}

determine_install_dir() {
    if [ -n "${WALRUS_INSTALL_DIR:-}" ]; then
        INSTALL_DIR="$WALRUS_INSTALL_DIR"
        return
    fi
    INSTALL_DIR="/usr/local/bin"
}

has_prebuilt() {
    case "${OS}-${ARCH}" in
        macos-arm64 | macos-amd64 | linux-amd64 | linux-arm64) return 0 ;;
        *) return 1 ;;
    esac
}

# Build the list of service binaries to install.
resolve_services() {
    if [ "$INSTALL_SERVICES" = "1" ]; then
        INSTALL_SERVICE_LIST="$SERVICE_BINS"
    fi
}

# --- Installation functions ---

ensure_cargo() {
    if check_cmd cargo; then
        return
    fi

    info "cargo not found, installing via rustup..."
    if ! check_cmd curl; then
        err "need 'curl' to install rustup"
    fi

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"

    if ! check_cmd cargo; then
        err "cargo still not found after rustup install"
    fi
    info "cargo installed successfully"
}

# Download and install a prebuilt binary from GitHub releases.
# Usage: install_binary <binary_name>
install_binary() {
    _bin="$1"
    TMPDIR_PATH="$(mktemp -d)"

    _tarball="${_bin}-${VERSION}-${OS}-${ARCH}.tar.gz"
    _url="https://github.com/${REPO}/releases/download/${VERSION}/${_tarball}"

    info "downloading ${_tarball}..."
    curl -fL# "$_url" -o "${TMPDIR_PATH}/${_tarball}"

    info "extracting..."
    tar -xzf "${TMPDIR_PATH}/${_tarball}" -C "${TMPDIR_PATH}"

    if [ ! -f "${TMPDIR_PATH}/${_bin}" ]; then
        err "expected binary '${_bin}' not found in tarball"
    fi

    # Place binary in install dir, handling permissions.
    if [ -w "$INSTALL_DIR" ]; then
        cp "${TMPDIR_PATH}/${_bin}" "${INSTALL_DIR}/${_bin}"
        chmod +x "${INSTALL_DIR}/${_bin}"
    elif confirm "use sudo to install to ${INSTALL_DIR}?"; then
        sudo cp "${TMPDIR_PATH}/${_bin}" "${INSTALL_DIR}/${_bin}"
        sudo chmod +x "${INSTALL_DIR}/${_bin}"
    else
        INSTALL_DIR="${HOME}/.local/bin"
        mkdir -p "$INSTALL_DIR"
        cp "${TMPDIR_PATH}/${_bin}" "${INSTALL_DIR}/${_bin}"
        chmod +x "${INSTALL_DIR}/${_bin}"
    fi

    info "installed ${_bin} to ${INSTALL_DIR}/${_bin}"
    rm -rf "$TMPDIR_PATH"
    TMPDIR_PATH=""
}

post_install() {
    # Find the installed binary.
    BIN_PATH=""
    if [ -n "${INSTALL_DIR:-}" ] && [ -x "${INSTALL_DIR}/${BINARY_NAME}" ]; then
        BIN_PATH="${INSTALL_DIR}/${BINARY_NAME}"
    elif check_cmd "$BINARY_NAME"; then
        BIN_PATH="$(command -v "$BINARY_NAME")"
    fi

    if [ -z "$BIN_PATH" ]; then
        warn "installation finished but '${BINARY_NAME}' not found in PATH"
        return
    fi

    # Check if the install dir is in PATH.
    case ":${PATH}:" in
        *":${INSTALL_DIR:-}:"*) ;;
        *)
            if [ -n "${INSTALL_DIR:-}" ]; then
                echo ""
                warn "${INSTALL_DIR} is not in your PATH"
                info "add it with:"
                printf "  export PATH=\"%s:\$PATH\"\n" "$INSTALL_DIR" >&2
                echo "" >&2
            fi
            ;;
    esac

    echo ""
    "$BIN_PATH" --help
}

cleanup() {
    if [ -n "${TMPDIR_PATH:-}" ] && [ -d "${TMPDIR_PATH}" ]; then
        rm -rf "$TMPDIR_PATH"
    fi
}

# --- Main ---

main() {
    parse_args "$@"
    setup_colors
    need_cmd curl
    need_cmd uname
    detect_platform
    detect_existing
    determine_install_dir
    resolve_services
    trap cleanup EXIT

    # --- Service-only install (no walrus binary) ---
    if [ -n "$INSTALL_SERVICE_LIST" ] && [ "$INSTALL_SERVICES" = "0" ]; then
        if has_prebuilt; then
            get_latest_version
            for svc in $INSTALL_SERVICE_LIST; do
                install_binary "$svc"
            done
        else
            warn "no prebuilt binary available for ${OS}-${ARCH}."
            info "falling back to cargo install..."
            ensure_cargo
            _crates=""
            for svc in $INSTALL_SERVICE_LIST; do
                _crates="${_crates} $(bin_to_crate "$svc")"
            done
            # shellcheck disable=SC2086
            cargo install $_crates
        fi
        info "service installation complete"
        return
    fi

    # --- Existing installation check ---
    if [ -n "$EXISTING_PATH" ]; then
        warn "${BINARY_NAME} is already installed at ${EXISTING_PATH}"
        if ! confirm "do you want to override it?"; then
            info "installation cancelled."
            exit 0
        fi
    fi

    # --- Prebuilt binary path ---
    if has_prebuilt; then
        get_latest_version
        install_binary "$BINARY_NAME"
        for svc in $INSTALL_SERVICE_LIST; do
            install_binary "$svc"
        done
        post_install
        return
    fi

    # --- Unsupported platform fallback ---
    warn "no prebuilt binary available for ${OS}-${ARCH}."
    info "falling back to cargo install..."
    ensure_cargo
    cargo install "$CARGO_CRATE"
    if [ -n "$INSTALL_SERVICE_LIST" ]; then
        _crates=""
        for svc in $INSTALL_SERVICE_LIST; do
            _crates="${_crates} $(bin_to_crate "$svc")"
        done
        # shellcheck disable=SC2086
        cargo install $_crates
    fi
    post_install
}

main "$@"
