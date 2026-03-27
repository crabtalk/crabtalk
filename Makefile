# Crabtalk Makefile
#
# Cross-platform builds for crabtalk CLI and extension services.
#
# Usage:
# make crabtalk   (CLI only, all platforms)
# make bundle     (CLI + services, all platforms)
# make macos-arm64
# make macos-amd64
# make linux-arm64
# make linux-amd64
# make windows-amd64
VERSION = v$(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
CARGO = cargo b --profile prod
PACKAGES = -p crabtalk -p crabtalk-search -p crabtalk-telegram
BINS = crabtalk crabtalk-search crabtalk-telegram

# Cross-compilation: set CC/AR so aws-lc-sys cmake uses the right
# assembler (macOS as doesn't understand armv8.4-a+sha3 etc).
LINUX_ARM64_ENV = CC=aarch64-linux-gnu-gcc AR=aarch64-linux-gnu-ar
LINUX_AMD64_ENV = CC=x86_64-linux-gnu-gcc AR=x86_64-linux-gnu-ar

# Per-platform cargo command prefix and target triple.
build-macos-arm64 = $(CARGO) --target aarch64-apple-darwin
build-macos-amd64 = CC_x86_64_apple_darwin=$(CURDIR)/.cargo/cc-x86_64.sh $(CARGO) --target x86_64-apple-darwin
build-linux-arm64 = $(LINUX_ARM64_ENV) $(CARGO) --target aarch64-unknown-linux-gnu
build-linux-amd64 = CC_x86_64_unknown_linux_gnu=$(CURDIR)/.cargo/cc-x86_64-linux.sh $(LINUX_AMD64_ENV) $(CARGO) --target x86_64-unknown-linux-gnu
build-windows-amd64 = $(CARGO) --target x86_64-pc-windows-msvc

triple-macos-arm64 = aarch64-apple-darwin
triple-macos-amd64 = x86_64-apple-darwin
triple-linux-arm64 = aarch64-unknown-linux-gnu
triple-linux-amd64 = x86_64-unknown-linux-gnu
triple-windows-amd64 = x86_64-pc-windows-msvc

# Binary extension per platform (empty on Unix, .exe on Windows).
ext-macos-arm64 =
ext-macos-amd64 =
ext-linux-arm64 =
ext-linux-amd64 =
ext-windows-amd64 = .exe

PLATFORMS = macos-arm64 macos-amd64 linux-amd64 linux-arm64 windows-amd64

# build only the crabtalk CLI for all platforms
crabtalk: $(addprefix crabtalk-,$(PLATFORMS))
	mkdir -p target/bundle
	$(foreach p,$(PLATFORMS),\
		tar -czf target/bundle/crabtalk-$(VERSION)-$(p).tar.gz -C target/$(triple-$(p))/prod crabtalk$(ext-$(p));)

crabtalk-%:
	$(build-$*) -p crabtalk

# build all packages for all platforms
bundle: $(PLATFORMS) tar-all

tar-all:
	mkdir -p target/bundle
	$(foreach bin,$(BINS),$(foreach p,$(PLATFORMS),\
		tar -czf target/bundle/$(bin)-$(VERSION)-$(p).tar.gz -C target/$(triple-$(p))/prod $(bin)$(ext-$(p));))

macos-arm64 macos-amd64 linux-arm64 linux-amd64 windows-amd64:
	$(build-$@) $(PACKAGES)
