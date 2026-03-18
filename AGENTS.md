> [!CAUTION]
> **Every commit MUST follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
> No exceptions. No excuses. Non-compliant commits will break automated releases.**
> Format: `type(scope): description` — see [Commit Convention](#commit-convention) below.

# AGENTS.md

Rust CLI tool (`agency`) that manages OpenCode agent permissions via a template+override system. Generates `permissions.jsonc` from `_templates/*.jsonc` + `_agents/*.jsonc`, validates round-trip correctness, and produces audit reports.

## Build / Lint / Test

```bash
cargo build                          # Debug build
cargo build --release                # Release build
cargo test                           # All tests
cargo test <test_name>               # Single test, e.g.: cargo test test_generate_permissions_succeeds
cargo test -- --nocapture             # Tests with stdout
cargo fmt                            # Format
cargo clippy -- -D warnings          # Lint (warnings are errors)
make check                           # fmt + clippy + test + generate-permissions + validate
make validate                        # Validate permissions.jsonc matches templates+agents
make generate-permissions            # Regenerate permissions.jsonc
make compare SOURCE=a TARGET=b       # Compare two permissions files
make report                          # Generate audit-report.md
make resolve AGENT=<name>            # Resolve one agent's permissions
make install                         # Build release + install to ~/.local/bin

agency can <agent> "<command>"              # Check permission (exit 0=allow, 1=deny)
agency can <agent> "<command>" --explain    # Show matching rule and rationale
agency can <agent> "<command>" -p path      # Use custom permissions file
agency resolve <agent> --trace             # Show permission provenance (which template each entry comes from)
make can AGENT=x-backend CMD="npm test"     # Via Makefile
make uninstall                       # Remove installed binary

# Use custom directories (global flags work with any subcommand)
agency --templates-dir custom-templates --agents-dir custom-agents generate-permissions
agency --agents-dir legacy-overrides validate
```

Binary name: `agency` (defined in `Cargo.toml` `[[bin]]`).

## Project Structure

```
src/
  main.rs            # CLI entry (clap), subcommand dispatch
  lib.rs             # Library crate re-exports (pub mod for all modules)
  models.rs          # All types: Sot, SotAgent, SotPermission, BashPermission, Template, Override, etc.
  config.rs          # Project configuration: CLI flags, agency.jsonc, defaults
  jsonc.rs           # JSONC file parser (jsonc-parser -> serde_json)
  loader.rs          # File I/O: load SOT, templates, agent files, discover categories
  resolver.rs        # Resolve template baseline + agent overrides -> final permissions
  validator.rs       # Round-trip validation: resolve() output vs permissions.jsonc
  generator.rs       # Compute deltas, build override JSONC, group classification
  sot_generator.rs   # Generate full permissions.jsonc from templates+agents pipeline
  comment_gen.rs     # Generate JSONC comment blocks (file header, category, agent)
  jsonc_writer.rs    # Format permission blocks and assemble JSONC document
  can.rs             # Fast permission checker: `can` subcommand with binary caching
  reporter.rs        # Audit report generation (audit-report.md)
  template_dag.rs    # Template DAG: petgraph-based inheritance graph, cycle detection, topological sort
_templates/          # 15 templates (9 leaf + 1 mixin + 5 root) (*.jsonc)
_agents/             # 6 example agent override files (*.jsonc)
_prds/               # PRD documents (DO NOT MODIFY)
schemas/             # JSON Schema files for template and agent validation
agency.jsonc         # Optional project configuration (directory paths)
permissions.jsonc    # Generated output (reproducible via make generate-permissions)
```

## Code Style

### Rust Edition & Dependencies

- Edition 2021. Key deps: `clap` (derive), `serde`/`serde_json`, `jsonc-parser`, `anyhow`, `indexmap`, `bincode`, `petgraph`.
- Library crate: `opencode_agency` (in `lib.rs`). Binary crate: `agency` (in `main.rs`). Use `opencode_agency::` for library imports in `main.rs`.

### Error Handling

- Use `anyhow::Result` for all fallible functions. Chain context with `.with_context(|| format!(...))`.
- Use `anyhow::bail!()` for early-return errors. Convert non-anyhow errors with `.map_err(|e| anyhow::anyhow!(...))`.
- Print errors to stderr (`eprintln!`) and `process::exit(1)` in `main()`.

### Types & Data Structures

- Use `IndexMap` (not `HashMap`) everywhere for deterministic ordering. Exceptions: `can.rs` uses `HashMap` for binary cache serialization; `template_dag.rs` uses `HashMap`/`HashSet` for petgraph node lookups and DFS visited tracking.
- `type Sot = IndexMap<String, SotAgent>` — top-level permissions map.
- `type CategoryMap = IndexMap<String, Vec<String>>` — category -> sorted agent names.
- `BashPermission` is `#[serde(untagged)]` enum: `Simple(String)` or `Detailed(IndexMap<String, String>)`.
- "Simple" vs "complex" categories: `is_simple_category()` checks `SIMPLE_CATEGORIES` const array.

### Naming

- Modules: `snake_case` matching file names.
- Types/enums: `PascalCase` (`SotPermission`, `BashPermission`, `ValidationReport`).
- Functions: `snake_case`. Public API functions prefixed descriptively (`load_`, `discover_`, `resolve`, `validate_all`, `generate_`).
- Constants: `UPPER_SNAKE_CASE` (`SIMPLE_CATEGORIES`, `GROUP_ORDER`).
- CLI subcommand handlers: `cmd_<name>()` in main.rs.

### Formatting & Style

- 2-space indent in generated JSONC output (via `indent()` helper).
- Standard `rustfmt` for Rust code.
- `#[allow(dead_code)]` used sparingly at module level for future-use public APIs.
- Sort collections for deterministic output: `sort_keys()`, `sort()`, `dedup()`.

### Imports

- Group: `std::` first, then external crates (`anyhow`, `clap`, `indexmap`, `serde`, `serde_json`), then `crate::` internal modules.
- Use specific imports from `crate::models::{Type1, Type2}` rather than glob imports (except in `#[cfg(test)]` modules where `use super::*` and `use crate::models::*` are acceptable).

### Testing

- Tests use `#[test]` with descriptive names: `test_<function>_<scenario>`.
- Integration tests use `env!("CARGO_MANIFEST_DIR")` for project root path.
- Helper functions in test modules: `make_template()`, `make_override()`, `make_delta()` etc.
- Assert patterns: `assert!(result.is_ok(), "msg: {:?}", result)`, `assert_eq!`, `assert!(str.contains(...))`.
- Self-validation pattern in `sot_generator`: generated output is parsed back and verified against expected data.

### Module Pattern

- Each module is a separate file declared in `lib.rs` via `pub mod name;`.
- Public functions are the module API; internal helpers are private (`fn`, not `pub fn`).
- Test modules are inline `#[cfg(test)] mod tests { ... }` at bottom of file.

### JSONC Files

- Templates have `$schema`, `$version`, `category`, `description`, `purpose`, optional `$extends`, optional `$doc`, and `baseline.permission`.
- `$extends` on templates: optional field declaring parent template(s). Can be `null` (root), a single string (`"$extends": "vcs-permissions"`), or an array of strings (`"$extends": ["vcs-permissions"]`). Mixin templates (like `vcs-permissions`) hold shared permission blocks and are not directly extended by agents.
- Overrides have `$schema`, `$version`, `agent`, `$extends`, optional `$doc`, and `overrides.permission.{add,remove}`.
- `$doc` block fields: `baseline_rationale`, `security_note` (templates); `agent_summary`, `override_rationale` (overrides).
- JSON Schemas for validation: templates use `https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json`, agents use `https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json`.

## Commit Convention

This repository uses **[Conventional Commits](https://conventionalcommits.org)** enforced by `semantic-release`.
Every commit message must follow the format:

```
type(scope): description
```

`scope` is optional. `description` must be lowercase and imperative.

### Types and version impact

| Type       | Version bump | Example                                     |
| ---------- | ------------ | ------------------------------------------- |
| `feat`     | **minor**    | `feat: add template inheritance via DAG`    |
| `fix`      | **patch**    | `fix: correct wildcard matching precedence` |
| `chore`    | none         | `chore: update .gitignore`                  |
| `docs`     | none         | `docs: clarify permission model`            |
| `style`    | none         | `style: reformat generated JSONC output`    |
| `refactor` | none         | `refactor: simplify resolver merge logic`   |
| `test`     | none         | `test: add binary cache invalidation tests` |
| `ci`       | none         | `ci: pin node version to 20`                |

### Breaking changes → major version bump

Add `BREAKING CHANGE:` in the commit **footer** (any type triggers a major bump):

```
feat(permissions): remove legacy allow-all mode

BREAKING CHANGE: the `allow_all` permission key is no longer supported.
```

### Valid commit message examples

```
feat(can): add --explain flag for permission decisions
fix(resolver): handle empty override blocks without panic
chore: remove unused commented-out block
docs(agents): add commit convention section
ci: add semantic-release workflow
```

### Ownership of CHANGELOG.md and version tags

- **Do NOT manually edit `CHANGELOG.md`** — it is owned and rewritten by `semantic-release`.
- **Do NOT manually create version tags** — tags (`v1.2.3`) are created automatically by
  `semantic-release` when a release-triggering commit is merged to `main`.
- The release workflow runs on every push to `main` and on `workflow_dispatch`.
  If no release-worthy commits are detected, semantic-release exits without creating a release.

## Boundaries

- Do NOT modify files in `_prds/`.
- Do NOT modify `PRD.md` or `ralphy-loop.local.md`.
- `permissions.jsonc` is generated — edit templates/overrides instead, then regenerate.
- `audit-report.md` is generated — run `make report` to update.
