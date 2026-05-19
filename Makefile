# Crabtalk Makefile
#
# Cross-platform builds for the crabtalk ecosystem.
#
# Usage:
# make release        (core binaries, all platforms)
# make bundle         (core + services, all platforms)
# make macos-arm64    (all packages, one platform)
VERSION = v$(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
CARGO = cargo b --profile prod

CORE_PACKAGES = -p crabup -p crabtalkd -p crabtalk-cli
CORE_BINS = crabup crabtalkd crabtalk

SERVICE_PACKAGES = -p crabtalk-search -p crabtalk-telegram
SERVICE_BINS = crabtalk-search crabtalk-telegram

ALL_PACKAGES = $(CORE_PACKAGES) $(SERVICE_PACKAGES)
ALL_BINS = $(CORE_BINS) $(SERVICE_BINS)

# TLS backend: native-tls on macOS, rustls on Linux/Windows.
tls-macos-arm64 =
tls-macos-amd64 =
tls-linux-arm64 = --no-default-features --features rustls
tls-linux-amd64 = --no-default-features --features rustls
tls-windows-amd64 = --no-default-features --features rustls

# Cross-compilation: set CC/AR so aws-lc-sys cmake uses the right
# assembler (macOS as doesn't understand armv8.4-a+sha3 etc).
LINUX_ARM64_ENV = CC=aarch64-linux-gnu-gcc AR=aarch64-linux-gnu-ar
LINUX_AMD64_ENV = CC=x86_64-linux-gnu-gcc AR=x86_64-linux-gnu-ar

# Per-platform cargo command prefix and target triple.
build-macos-arm64 = $(CARGO) --target aarch64-apple-darwin
build-macos-amd64 = CC_x86_64_apple_darwin=$(CURDIR)/.cargo/cc-x86_64.sh $(CARGO) --target x86_64-apple-darwin
build-linux-arm64 = $(LINUX_ARM64_ENV) $(CARGO) --target aarch64-unknown-linux-gnu
build-linux-amd64 = CC_x86_64_unknown_linux_gnu=$(CURDIR)/.cargo/cc-x86_64-linux.sh $(LINUX_AMD64_ENV) $(CARGO) --target x86_64-unknown-linux-gnu
build-windows-amd64 = CC=x86_64-w64-mingw32-gcc AR=x86_64-w64-mingw32-ar $(CARGO) --target x86_64-pc-windows-gnu

triple-macos-arm64 = aarch64-apple-darwin
triple-macos-amd64 = x86_64-apple-darwin
triple-linux-arm64 = aarch64-unknown-linux-gnu
triple-linux-amd64 = x86_64-unknown-linux-gnu
triple-windows-amd64 = x86_64-pc-windows-gnu

# Binary extension per platform (empty on Unix, .exe on Windows).
ext-macos-arm64 =
ext-macos-amd64 =
ext-linux-arm64 =
ext-linux-amd64 =
ext-windows-amd64 = .exe

PLATFORMS = macos-arm64 macos-amd64 linux-amd64 linux-arm64 windows-amd64

# Build core binaries for all platforms, produce per-binary tarballs.
release: $(addprefix release-,$(PLATFORMS))
	mkdir -p target/bundle
	$(foreach bin,$(CORE_BINS),$(foreach p,$(PLATFORMS),\
		tar -czf target/bundle/$(bin)-$(VERSION)-$(p).tar.gz -C target/$(triple-$(p))/prod $(bin)$(ext-$(p));))

release-%:
	$(build-$*) $(CORE_PACKAGES) $(tls-$*)

# Build all packages (core + services) for all platforms.
bundle: $(PLATFORMS) tar-all

tar-all:
	mkdir -p target/bundle
	$(foreach bin,$(ALL_BINS),$(foreach p,$(PLATFORMS),\
		tar -czf target/bundle/$(bin)-$(VERSION)-$(p).tar.gz -C target/$(triple-$(p))/prod $(bin)$(ext-$(p));))

macos-arm64 macos-amd64 linux-arm64 linux-amd64 windows-amd64:
	$(build-$@) $(ALL_PACKAGES) $(tls-$@)

# Create a GitHub release and upload all core tarballs.
# Assumes `make release` has been run and tarballs exist in target/bundle/.
publish:
	gh release create $(VERSION) --title "$(VERSION)" --generate-notes target/bundle/*-$(VERSION)-*.tar.gz
