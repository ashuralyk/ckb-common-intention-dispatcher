# We cannot use $(shell pwd), which will return unix path format on Windows,
# making it hard to use.
cur_dir = $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

TOP := $(cur_dir)
CUSTOM_RUSTFLAGS := -C debug-assertions=yes
CARGO_ARGS := $(CARGO_ARGS)
MODE := release
CLANG := $(shell $(TOP)/scripts/find_clang)
CONTRACT :=
CLEAN_BUILD_DIR_FIRST := true
BUILD_DIR := build/$(MODE)

export CUSTOM_RUSTFLAGS
export TOP
export CARGO_ARGS
export MODE
export CLANG
export BUILD_DIR

default: build test

build:
	@if [ "x$(CLEAN_BUILD_DIR_FIRST)" = "xtrue" ]; then \
		echo "Cleaning $(BUILD_DIR) directory..."; \
		rm -rf $(BUILD_DIR); \
	fi
	mkdir -p $(BUILD_DIR)
	@set -eu; \
	if [ "x$(CONTRACT)" = "x" ]; then \
		for contract in $(wildcard contracts/*); do \
			$(MAKE) -e -C $$contract build; \
		done; \
	else \
		$(MAKE) -e -C contracts/$(CONTRACT) build; \
	fi

test:
	cd tests && cargo test $(CARGO_ARGS) -- --nocapture

# Expected sha256 of `ckb_common_intention_dispatcher` after `make build`.
# Filter/Sequencer work must not change this binary (CID Type ID redeploy).
CID_LOCK_SHA256 := 569f44461fb1c135d1c5d8e258c3a06e1a93fa30e6c44d2bd12d7702bec18c3e

check-cid-hash:
	@test -f build/release/ckb_common_intention_dispatcher || { \
		echo "missing build/release/ckb_common_intention_dispatcher — run make build first"; \
		exit 1; \
	}
	@echo "$(CID_LOCK_SHA256)  build/release/ckb_common_intention_dispatcher" | sha256sum --check

prepare:
	rustup target add riscv64imac-unknown-none-elf

clean:
	rm -rf build
	cargo clean

.PHONY: build test check-cid-hash prepare clean
