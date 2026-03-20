#!/bin/sh
# Install script for crabtalk — https://github.com/crabtalk/crabtalk
#
# Usage:
#   curl -fsSL crabtalk.ai/install | sh
#   curl -fsSL crabtalk.ai/install | sh -s -- --yes   # non-interactive
#
# Environment variables:
#   CRABTALK_INSTALL_DIR  Override binary installation directory

set -eu

REPO="crabtalk/crabtalk"
BINARY_NAME="crabtalk"
CARGO_CRATE="crabtalk"
AUTO_YES=0
TMPDIR_PATH=""

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

# --- Detection functions ---

parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --yes | -y)
                AUTO_YES=1
                ;;
            --help | -h)
                cat <<'EOF'
Install crabtalk — composable primitives for agentic workflows in Rust.

Usage:
  install.sh [OPTIONS]

Options:
  -y, --yes              Skip all confirmation prompts (downloads prebuilt binary)
  -h, --help             Show this help message

Environment variables:
  CRABTALK_INSTALL_DIR   Override binary installation directory
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
    if [ -n "${CRABTALK_INSTALL_DIR:-}" ]; then
        INSTALL_DIR="$CRABTALK_INSTALL_DIR"
        return
    fi
    if [ -w "/usr/local/bin" ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="${HOME}/.local/bin"
    fi
}

has_prebuilt() {
    case "${OS}-${ARCH}" in
        macos-arm64 | macos-amd64 | linux-amd64 | linux-arm64) return 0 ;;
        *) return 1 ;;
    esac
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
install_binary() {
    TMPDIR_PATH="$(mktemp -d)"

    _tarball="${BINARY_NAME}-${VERSION}-${OS}-${ARCH}.tar.gz"
    _url="https://github.com/${REPO}/releases/download/${VERSION}/${_tarball}"

    info "downloading ${_tarball}..."
    curl -fL# "$_url" -o "${TMPDIR_PATH}/${_tarball}"

    info "extracting..."
    tar -xzf "${TMPDIR_PATH}/${_tarball}" -C "${TMPDIR_PATH}"

    if [ ! -f "${TMPDIR_PATH}/${BINARY_NAME}" ]; then
        err "expected binary '${BINARY_NAME}' not found in tarball"
    fi

    mkdir -p "$INSTALL_DIR"
    cp "${TMPDIR_PATH}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    info "installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"
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

    # Ensure the install dir is in PATH.
    case ":${PATH}:" in
        *":${INSTALL_DIR:-}:"*) ;;
        *)
            if [ -n "${INSTALL_DIR:-}" ]; then
                _export_line="export PATH=\"${INSTALL_DIR}:\$PATH\""
                echo ""
                warn "${INSTALL_DIR} is not in your PATH"

                # Detect the user's shell profile.
                _profile=""
                case "${SHELL:-}" in
                    */zsh)  _profile="$HOME/.zshrc" ;;
                    */bash)
                        if [ -f "$HOME/.bashrc" ]; then
                            _profile="$HOME/.bashrc"
                        elif [ -f "$HOME/.bash_profile" ]; then
                            _profile="$HOME/.bash_profile"
                        fi
                        ;;
                    */fish) _profile="$HOME/.config/fish/config.fish" ;;
                esac

                if [ -n "$_profile" ] && confirm "add ${INSTALL_DIR} to PATH in ${_profile}?"; then
                    echo "$_export_line" >> "$_profile"
                    info "added to ${_profile} — restart your shell or run:"
                    printf "  %s\n" "$_export_line" >&2
                else
                    info "add it manually:"
                    printf "  %s\n" "$_export_line" >&2
                fi

                # Make it available for the rest of this script.
                export PATH="${INSTALL_DIR}:$PATH"
                echo "" >&2
            fi
            ;;
    esac

    echo ""
    "$BIN_PATH" --help

    # Offer to start the daemon (includes auth setup on first run).
    echo ""
    if confirm "start the daemon now?"; then
        "$BIN_PATH" daemon start
    fi
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
    trap cleanup EXIT

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
        install_binary
        post_install
        return
    fi

    # --- Unsupported platform fallback ---
    warn "no prebuilt binary available for ${OS}-${ARCH}."
    info "falling back to cargo install..."
    ensure_cargo
    cargo install "$CARGO_CRATE"
    post_install
}

main "$@"
