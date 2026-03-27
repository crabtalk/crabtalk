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

NVM_INSTALL_URL="https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh"
CRABTALK_ENV="${HOME}/.crabtalk/env"

# State set by find_* / ensure_* functions.
RUST_FOUND=0
CARGO_BIN_DIR=""
NODE_FOUND=0
NVM_FOUND=0
NODE_BIN_DIR=""

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

# Check whether a directory is already in PATH.
in_path() {
    case ":${PATH}:" in
        *":$1:"*) return 0 ;;
        *)        return 1 ;;
    esac
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
    case "$OS" in
        windows)
            INSTALL_DIR="${LOCALAPPDATA:-$HOME/AppData/Local}/crabtalk/bin"
            ;;
        *)
            if [ -w "/usr/local/bin" ]; then
                INSTALL_DIR="/usr/local/bin"
            else
                INSTALL_DIR="${HOME}/.local/bin"
            fi
            ;;
    esac
}

has_prebuilt() {
    case "${OS}-${ARCH}" in
        macos-arm64 | macos-amd64 | linux-amd64 | linux-arm64 | windows-amd64) return 0 ;;
        *) return 1 ;;
    esac
}

# --- Toolchain detection ---

# Locate nvm.sh from well-known paths. Sets _nvm_sh if found.
locate_nvm_sh() {
    _nvm_sh=""
    for _dir in "${NVM_DIR:-}" "$HOME/.nvm" "$HOME/.local/share/nvm"; do
        if [ -n "$_dir" ] && [ -s "$_dir/nvm.sh" ]; then
            _nvm_sh="$_dir/nvm.sh"
            return 0
        fi
    done
    return 1
}

# Source nvm into the current shell if not already loaded.
source_nvm() {
    if check_cmd nvm; then
        return 0
    fi
    if locate_nvm_sh; then
        NVM_DIR="$(dirname "$_nvm_sh")"
        export NVM_DIR
        # shellcheck disable=SC1090
        . "$_nvm_sh"
        return 0
    fi
    return 1
}

find_rust() {
    RUST_FOUND=0
    CARGO_BIN_DIR=""

    # Already in PATH.
    if check_cmd cargo; then
        RUST_FOUND=1
        CARGO_BIN_DIR="$(dirname "$(command -v cargo)")"
        info "found cargo at $(command -v cargo)"
        return
    fi

    # Check well-known locations.
    _cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    if [ -x "$_cargo_home/bin/cargo" ]; then
        RUST_FOUND=1
        CARGO_BIN_DIR="$_cargo_home/bin"
        info "found cargo at $_cargo_home/bin/cargo (not in PATH)"
        return
    fi
}

find_node() {
    NODE_FOUND=0
    NVM_FOUND=0
    NODE_BIN_DIR=""

    # Already in PATH.
    if check_cmd node; then
        NODE_FOUND=1
        NODE_BIN_DIR="$(dirname "$(command -v node)")"
        info "found node at $(command -v node)"
        return
    fi

    # Try sourcing nvm to pick up an installed node.
    if source_nvm; then
        NVM_FOUND=1
        if check_cmd node; then
            NODE_FOUND=1
            NODE_BIN_DIR="$(dirname "$(command -v node)")"
            info "found node via nvm at $(command -v node)"
            return
        fi
        info "found nvm but no node version installed"
        return
    fi
}

# --- Installation functions ---

ensure_rust() {
    if [ "$RUST_FOUND" = "1" ]; then
        return
    fi

    if ! confirm "cargo is required to build crabtalk on this platform. install rust via rustup?"; then
        err "cannot proceed without cargo"
    fi

    info "installing rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"

    if ! check_cmd cargo; then
        err "cargo still not found after rustup install"
    fi

    RUST_FOUND=1
    CARGO_BIN_DIR="$HOME/.cargo/bin"
    info "cargo installed successfully"
}

ensure_node() {
    if [ "$NODE_FOUND" = "1" ]; then
        return
    fi

    # nvm exists but no node — just need to install a version.
    if [ "$NVM_FOUND" = "1" ]; then
        if ! confirm "nvm is installed but no node version found. install node LTS for crabtalk skills (npx)?"; then
            warn "skills requiring npx will not work without node"
            return
        fi

        nvm install --lts
        nvm use --lts

        NODE_FOUND=1
        NODE_BIN_DIR="$(dirname "$(command -v node)")"
        info "node $(node --version) installed via nvm"
        return
    fi

    # Nothing found — install nvm + node.
    if ! confirm "node.js is required to run crabtalk skills (npx). install via nvm?"; then
        warn "skills requiring npx will not work without node"
        return
    fi

    info "installing nvm..."
    curl -fsSL "$NVM_INSTALL_URL" | PROFILE=/dev/null bash

    export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
    # shellcheck disable=SC1091
    . "$NVM_DIR/nvm.sh"

    info "installing node LTS..."
    nvm install --lts
    nvm use --lts

    if ! check_cmd node; then
        err "node still not found after nvm install"
    fi

    NODE_FOUND=1
    NVM_FOUND=1
    NODE_BIN_DIR="$(dirname "$(command -v node)")"
    info "node $(node --version) installed via nvm"
}

# --- Environment file ---

write_crabtalk_env() {
    _env_paths=""

    # Collect directories that need to be in PATH.
    if [ -n "$CARGO_BIN_DIR" ] && ! in_path "$CARGO_BIN_DIR"; then
        _env_paths="$CARGO_BIN_DIR"
    fi

    if [ -n "${INSTALL_DIR:-}" ] && ! in_path "$INSTALL_DIR"; then
        if [ -n "$_env_paths" ]; then
            _env_paths="$INSTALL_DIR:$_env_paths"
        else
            _env_paths="$INSTALL_DIR"
        fi
    fi

    # For nvm-managed node we source nvm.sh instead of hardcoding the bin path,
    # because the path changes with every node version update.
    _nvm_source=""
    if [ "$NVM_FOUND" = "1" ]; then
        _nvm_source="export NVM_DIR=\"\${NVM_DIR:-\$HOME/.nvm}\"
[ -s \"\$NVM_DIR/nvm.sh\" ] && . \"\$NVM_DIR/nvm.sh\""
    elif [ -n "$NODE_BIN_DIR" ] && ! in_path "$NODE_BIN_DIR"; then
        # Node installed without nvm (system package, etc.) — add to PATH directly.
        if [ -n "$_env_paths" ]; then
            _env_paths="$NODE_BIN_DIR:$_env_paths"
        else
            _env_paths="$NODE_BIN_DIR"
        fi
    fi

    # Nothing to write.
    if [ -z "$_env_paths" ] && [ -z "$_nvm_source" ]; then
        return
    fi

    mkdir -p "$(dirname "$CRABTALK_ENV")"

    {
        echo "# crabtalk environment — sourced by shell profile"
        echo "# Generated by install.sh — edits may be overwritten."
        if [ -n "$_env_paths" ]; then
            echo "export PATH=\"$_env_paths:\$PATH\""
        fi
        if [ -n "$_nvm_source" ]; then
            echo "$_nvm_source"
        fi
    } > "$CRABTALK_ENV"

    info "wrote ${CRABTALK_ENV}"
}

setup_shell_profile() {
    if [ ! -f "$CRABTALK_ENV" ]; then
        return
    fi

    _source_line=". \"$CRABTALK_ENV\""

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

    if [ -z "$_profile" ]; then
        warn "could not detect shell profile"
        info "add this to your shell profile manually:"
        printf "  %s\n" "$_source_line" >&2
        return
    fi

    # Already present.
    if [ -f "$_profile" ] && grep -qF ".crabtalk/env" "$_profile" 2>/dev/null; then
        return
    fi

    echo ""
    if confirm "add crabtalk environment to ${_profile}?"; then
        # fish uses different source syntax.
        if [ "${SHELL:-}" != "${SHELL%fish}" ]; then
            echo "source \"$CRABTALK_ENV\"" >> "$_profile"
        else
            echo "$_source_line" >> "$_profile"
        fi
        info "added to ${_profile} — restart your shell or run:"
        printf "  %s\n" "$_source_line" >&2
    else
        info "add it manually:"
        printf "  %s\n" "$_source_line" >&2
    fi

    # Source it now so the rest of this script can use the paths.
    # shellcheck disable=SC1090
    . "$CRABTALK_ENV"
}

# Download and install a prebuilt binary from GitHub releases.
install_binary() {
    TMPDIR_PATH="$(mktemp -d)"

    # Windows binaries have .exe extension.
    _ext=""
    if [ "$OS" = "windows" ]; then
        _ext=".exe"
    fi

    _tarball="${BINARY_NAME}-${VERSION}-${OS}-${ARCH}.tar.gz"
    _url="https://github.com/${REPO}/releases/download/${VERSION}/${_tarball}"

    info "downloading ${_tarball}..."
    curl -fL# "$_url" -o "${TMPDIR_PATH}/${_tarball}"

    info "extracting..."
    tar -xzf "${TMPDIR_PATH}/${_tarball}" -C "${TMPDIR_PATH}"

    if [ ! -f "${TMPDIR_PATH}/${BINARY_NAME}${_ext}" ]; then
        err "expected binary '${BINARY_NAME}${_ext}' not found in tarball"
    fi

    mkdir -p "$INSTALL_DIR"
    cp "${TMPDIR_PATH}/${BINARY_NAME}${_ext}" "${INSTALL_DIR}/${BINARY_NAME}${_ext}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}${_ext}" 2>/dev/null || true

    info "installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}${_ext}"
    rm -rf "$TMPDIR_PATH"
    TMPDIR_PATH=""
}

post_install() {
    # Find the installed binary.
    _ext=""
    if [ "$OS" = "windows" ]; then
        _ext=".exe"
    fi

    BIN_PATH=""
    if [ -n "${INSTALL_DIR:-}" ] && [ -x "${INSTALL_DIR}/${BINARY_NAME}${_ext}" ]; then
        BIN_PATH="${INSTALL_DIR}/${BINARY_NAME}${_ext}"
    elif check_cmd "$BINARY_NAME"; then
        BIN_PATH="$(command -v "$BINARY_NAME")"
    fi

    if [ -z "$BIN_PATH" ]; then
        warn "installation finished but '${BINARY_NAME}' not found in PATH"
        return
    fi

    # Set up toolchain environment.
    ensure_node
    write_crabtalk_env
    setup_shell_profile

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

    # Detect available toolchains.
    find_rust
    find_node

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
    ensure_rust
    cargo install "$CARGO_CRATE"
    post_install
}

main "$@"
