# Crabtalk Makefile

# ── Harnesses ────────────────────────────────────────────────────
#
# A harness is one RV64 ELF rather than a binary per platform. `make harness`
# builds them and installs them where the daemon looks; the daemon loads what
# is there and never fetches.
#
# CRABTALK_HOME follows the configuration directory rather than being written
# out, so pointing crabtalk somewhere else points this there too.
#
# Needs the harness target: rustup target add riscv64imac-unknown-none-elf
CRABTALK_HOME ?= $(HOME)/.crabtalk
HARNESS_TARGET = riscv64imac-unknown-none-elf
HARNESS_DIR = $(CRABTALK_HOME)/harnesses
HARNESSES = os peers sessions skill

harness:
	cargo build --release --target $(HARNESS_TARGET) \
		$(foreach h,$(HARNESSES),-p berm-$(h))
	mkdir -p $(HARNESS_DIR)
	$(foreach h,$(HARNESSES),\
		cp target/$(HARNESS_TARGET)/release/$(h) $(HARNESS_DIR)/$(h).elf;)

.PHONY: harness
