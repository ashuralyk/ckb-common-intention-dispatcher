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

# Expected sha256 of `ckb_common_intention_dispatcher` after `make build` in the
# known deploy / agent environment. Filter/Sequencer work must not change this
# binary (CID Type ID redeploy). GitHub Actions may produce a different absolute
# hash for the same source — CI uses `check-cid-unchanged` (same-toolchain
# compare against the PR base) instead of this absolute pin.
CID_LOCK_SHA256 := 569f44461fb1c135d1c5d8e258c3a06e1a93fa30e6c44d2bd12d7702bec18c3e

check-cid-hash:
	@test -f build/release/ckb_common_intention_dispatcher || { \
		echo "missing build/release/ckb_common_intention_dispatcher — run make build first"; \
		exit 1; \
	}
	@if echo "$(CID_LOCK_SHA256)  build/release/ckb_common_intention_dispatcher" | sha256sum --check; then \
		: ; \
	else \
		echo "actual: $$(sha256sum build/release/ckb_common_intention_dispatcher)"; \
		echo "expected (deploy/agent pin): $(CID_LOCK_SHA256)"; \
		exit 1; \
	fi

# Compare the already-built CID lock binary to one rebuilt from BASE_SHA with
# the same CLANG / CARGO_ARGS. Usage:
#   make build CLANG=clang-18 CARGO_ARGS=--locked
#   make check-cid-unchanged BASE_SHA=<git-sha> CLANG=clang-18 CARGO_ARGS=--locked
BASE_SHA ?=
check-cid-unchanged:
	@test -n "$(BASE_SHA)" || { echo "BASE_SHA is required"; exit 1; }
	@test -f build/release/ckb_common_intention_dispatcher || { \
		echo "missing build/release/ckb_common_intention_dispatcher — run make build first"; \
		exit 1; \
	}
	@set -eu; \
	CURRENT=$$(sha256sum build/release/ckb_common_intention_dispatcher | awk '{print $$1}'); \
	echo "current CID sha256=$$CURRENT"; \
	WT=$$(mktemp -d /tmp/cid-base-XXXXXX); \
	cleanup() { git worktree remove --force "$$WT" 2>/dev/null || rm -rf "$$WT"; }; \
	trap cleanup EXIT; \
	git worktree add --detach "$$WT" "$(BASE_SHA)"; \
	$(MAKE) -C "$$WT" build CONTRACT=ckb_common_intention_dispatcher \
		CLANG="$(CLANG)" CARGO_ARGS="$(CARGO_ARGS)"; \
	BASE=$$(sha256sum "$$WT/build/release/ckb_common_intention_dispatcher" | awk '{print $$1}'); \
	echo "base ($(BASE_SHA)) CID sha256=$$BASE"; \
	test "$$CURRENT" = "$$BASE" || { \
		echo "CID lock binary changed vs $(BASE_SHA) — filter/Sequencer work must not alter it"; \
		exit 1; \
	}; \
	echo "CID lock binary unchanged vs $(BASE_SHA)"

prepare:
	rustup target add riscv64imac-unknown-none-elf

clean:
	rm -rf build
	cargo clean

.PHONY: build test check-cid-hash check-cid-unchanged prepare clean
