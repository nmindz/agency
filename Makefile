# Makefile for agency (OpenCode Agency)

BINARY := agency
INSTALL_DIR := $(HOME)/.local/bin
CARGO := cargo

.PHONY: all build release install uninstall test validate generate generate-permissions compare report resolve can clean fmt clippy check help

# Default target
all: release generate-permissions validate

build:
	$(CARGO) build

release:
	$(CARGO) build --release

install: release
	@mkdir -p $(INSTALL_DIR)
	cp target/release/$(BINARY) $(INSTALL_DIR)/$(BINARY)
	@echo "✓ Installed $(BINARY) to $(INSTALL_DIR)/$(BINARY)"

uninstall:
	rm -f $(INSTALL_DIR)/$(BINARY)
	@echo "✓ Removed $(BINARY) from $(INSTALL_DIR)"

test:
	$(CARGO) test

validate:
	$(CARGO) run -- validate

generate-permissions:
	$(CARGO) run -- generate-permissions

compare:
	$(CARGO) run -- compare $(SOURCE) $(TARGET)

generate:
	@echo "NOTE: 'make generate' is deprecated. Use 'make generate-permissions' instead."
	$(CARGO) run -- generate-overrides

report:
	$(CARGO) run -- audit-report

resolve:
	$(CARGO) run -- resolve $(AGENT)

can:
	$(CARGO) run -- can $(AGENT) "$(CMD)" $(if $(PERMISSIONS),--permissions $(PERMISSIONS),) $(if $(EXPLAIN),--explain,)

clean:
	$(CARGO) clean

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy -- -D warnings

check: fmt clippy test generate-permissions validate

help:
	@echo "agency — OpenCode Agency"
	@echo ""
	@echo "Targets:"
	@echo "  make build                    cargo build"
	@echo "  make release                  cargo build --release"
	@echo "  make install                  build release and install to ~/.local/bin"
	@echo "  make uninstall                remove installed binary from ~/.local/bin"
	@echo "  make test                     cargo test"
	@echo "  make generate-permissions     generate permissions.jsonc from templates + agents"
	@echo "  make validate                 validate permissions.jsonc against templates + agents"
	@echo "  make compare SOURCE=a TARGET=b   compare two permissions files"
	@echo "  make report                   generate audit-report.md"
	@echo "  make resolve AGENT=x          resolve one agent's permissions"
	@echo "  make can AGENT=x CMD='npm test'   check agent permission (exit 0=allow, 1=deny)"
	@echo "  make clean                    cargo clean"
	@echo "  make fmt                      cargo fmt"
	@echo "  make clippy                   cargo clippy -- -D warnings"
	@echo "  make check                    fmt + clippy + test + generate-permissions + validate"
	@echo "  make all                      release + generate-permissions + validate (default)"
	@echo "  make help                     print this help"
	@echo ""
	@echo "Deprecated:"
	@echo "  make generate                 [deprecated] generate overrides from SOT"
