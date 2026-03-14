# Walrus Makefile
#
# Cross-platform builds for walrus (CLI with daemon feature).
#
# - macOS Apple Silicon: Metal acceleration
# - linux x86_64: CUDA acceleration
#
# Usage:
# make bundle
# make macos-arm64
# make macos-amd64
# make linux-arm64
# make linux-amd64
VERSION = v0.0.4
CARGO = cargo b --profile prod

# Cross-compilation: set CC/AR so aws-lc-sys cmake uses the right
# assembler (macOS as doesn't understand armv8.4-a+sha3 etc).
LINUX_ARM64_ENV = CC=aarch64-linux-gnu-gcc AR=aarch64-linux-gnu-ar
LINUX_AMD64_ENV = CC=x86_64-linux-gnu-gcc AR=x86_64-linux-gnu-ar

# build all targets
bundle: macos-arm64 macos-amd64 linux-amd64 linux-arm64 tar-all

# make tarballs for all binaries
tar-all: tar-walrus

# make tarballs for walrus
tar-walrus:
	mkdir -p target/bundle
	tar -czf target/bundle/walrus-$(VERSION)-macos-arm64.tar.gz -C target/aarch64-apple-darwin/prod walrus
	tar -czf target/bundle/walrus-$(VERSION)-macos-amd64.tar.gz -C target/x86_64-apple-darwin/prod walrus
	tar -czf target/bundle/walrus-$(VERSION)-linux-amd64.tar.gz -C target/x86_64-unknown-linux-gnu/prod walrus
	tar -czf target/bundle/walrus-$(VERSION)-linux-arm64.tar.gz -C target/aarch64-unknown-linux-gnu/prod walrus

# build macos-arm64 (Metal acceleration)
macos-arm64:
	$(CARGO) --target aarch64-apple-darwin -p openwalrus

# build macos-amd64
# .cargo/cc-x86_64.sh rewrites -march=native (ARM host) to x86-64-v4
# so lance-linalg AVX-512 C kernels compile correctly when cross-compiling.
macos-amd64:
	CC_x86_64_apple_darwin=$(CURDIR)/.cargo/cc-x86_64.sh $(CARGO) --target x86_64-apple-darwin -p openwalrus

# build linux-arm64
linux-arm64:
	$(LINUX_ARM64_ENV) $(CARGO) --target aarch64-unknown-linux-gnu -p openwalrus

# build linux-amd64 (CUDA acceleration)
# CC_x86_64_unknown_linux_gnu rewrites -march=native (ARM host) to x86-64-v4
# so lance-linalg AVX-512 C kernels compile correctly when cross-compiling.
linux-amd64:
	CC_x86_64_unknown_linux_gnu=$(CURDIR)/.cargo/cc-x86_64-linux.sh $(LINUX_AMD64_ENV) $(CARGO) --target x86_64-unknown-linux-gnu -p openwalrus
