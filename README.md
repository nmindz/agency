<p align="center">
  <img src="logo.png" width="256" alt="Agency" />
</p>

<h1 align="center">Agency</h1>

<p align="center">
  <strong>Agent permission manager for OpenCode</strong><br/>
  Template + override system that generates self-documenting <code>permissions.jsonc</code> from structured metadata.
</p>

<p align="center">
  <a href="#architecture">Architecture</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#commands">Commands</a> •
  <a href="#project-structure">Project Structure</a> •
  <a href="#the-doc-system">The $doc System</a> •
  <a href="#development">Development</a>
</p>

---

## Architecture

Agency manages permissions for **6 example agents** across **15 templates (9 leaf + 1 mixin + 5 root)** using a template + override system:

```
_templates/ + _agents/  (Source of Truth)
         │
         ▼
    agency generate-permissions
         │
         ▼
  permissions.jsonc  (Generated, self-documenting)
```

**Templates** define category-level baselines (e.g., "backend-api agents get Node.js, TypeScript, linters, Make, and VCS access").

**Agent files** define per-agent deviations (e.g., "x-backend-engineer additionally gets Docker, Playwright, and Turbo").

The generated `permissions.jsonc` includes human-readable JSONC comments sourced entirely from `$doc` metadata fields — zero hardcoded descriptions.

## Quick Start

```bash
# Build
cargo build

# Generate permissions.jsonc from templates + agent files
agency generate-permissions

# Validate round-trip correctness (all agents must pass)
agency validate

# Compare two permissions files
agency compare permissions.jsonc other-permissions.jsonc

# Full check: format + lint + test + generate + validate
make check
```

## Configuration

Agency uses the following priority for directory paths:

1. **CLI flags**: `--templates-dir`, `--agents-dir`
2. **Config file**: `agency.jsonc` in project root
3. **Defaults**: `_templates/`, `_agents/`

### Config File (`agency.jsonc`)

Optional. All fields are optional — missing fields use defaults.

```jsonc
{
  "templates_dir": "_templates",
  "agents_dir": "_agents",
}
```

### CLI Flags

```bash
# Use custom directories
agency --templates-dir custom-templates --agents-dir custom-agents generate-permissions

# Flags work with any subcommand
agency --agents-dir legacy-overrides validate
```

### Error Handling

If a directory doesn't exist, you'll get a helpful message:

```
Error: Agents directory not found: /path/to/_agents
  Hint: Create the directory or specify a custom path with --agents-dir
```

## Installation

```bash
# Build and install to ~/.local/bin
make install

# Or manually
cargo build --release
cp target/release/agency ~/.local/bin/
```

Make sure `~/.local/bin` is in your `PATH`.

## Commands

| Command                            | Description                                                               |
| ---------------------------------- | ------------------------------------------------------------------------- |
| `agency generate-permissions`      | Generate `permissions.jsonc` from `_templates/` + `_agents/`              |
| `agency compare <source> <target>` | Compare two permissions files agent-by-agent                              |
| `agency validate`                  | Validate that `permissions.jsonc` matches what templates + agents produce |
| `agency resolve <agent>`           | Show resolved permissions for a specific agent (`--trace` for provenance) |
| `agency audit-report`              | Generate `audit-report.md` with delta analysis                            |
| `agency can <agent> "<cmd>"`       | Check if agent can run command (exit 0=allow, 1=deny)                     |
| `agency generate-overrides`        | _(Deprecated)_ Generate override files from SOT                           |

### Options

```bash
# Custom output path
agency generate-permissions --output /path/to/output.jsonc

# Compare two files
agency compare permissions.jsonc other-permissions.jsonc

# Check permission with explanation
agency can x-backend-engineer "npm test" --explain

# Use custom permissions file
agency can x-backend-engineer "npm test" -p /path/to/permissions.jsonc

# Show permission provenance (which template each entry comes from)
agency resolve x-backend-engineer --trace
```

> **Note:** Global flags `--templates-dir` and `--agents-dir` can be used with any subcommand. See [Configuration](#configuration).

## Project Structure

```
src/
  main.rs            # CLI entry (clap), subcommand dispatch
  lib.rs             # Library crate re-exports (pub mod for all modules)
  models.rs          # Types: Sot, SotAgent, SotPermission, Template, Override, DocBlock, etc.
  config.rs          # Project configuration: CLI flags, agency.jsonc, defaults
  jsonc.rs           # JSONC file parser (jsonc-parser → serde_json)
  loader.rs          # File I/O: load templates, agent files, discover categories
  resolver.rs        # Resolve template baseline + agent overrides → final permissions
  validator.rs       # Round-trip validation: resolve() output vs permissions.jsonc
  generator.rs       # Compute deltas, build override JSONC, group classification
  sot_generator.rs   # Generate full permissions.jsonc from templates + agents pipeline
  comment_gen.rs     # Generate JSONC comment blocks from $doc metadata
  jsonc_writer.rs    # Format permission blocks and assemble JSONC document
  can.rs             # Fast permission checker: `can` subcommand with binary caching
  reporter.rs        # Audit report generation (audit-report.md)
  template_dag.rs    # Template DAG: petgraph-based inheritance graph, cycle detection, topological sort
_templates/          # 15 templates (9 leaf + 1 mixin + 5 root) (*.jsonc)
_agents/             # 6 example agent override files (*.jsonc)
_prds/               # PRD documents (implementation specs)
schemas/             # JSON Schema files for template and agent validation
agency.jsonc         # Optional project configuration (directory paths)
permissions.jsonc    # Generated output (via `agency generate-permissions`)
audit-report.md      # Generated audit report (via `agency audit-report`)
```

## The `$doc` System

Every template and override file contains an optional `$doc` metadata block that flows through the generator into human-readable JSONC comments.

### Template `$doc`

```jsonc
// In _templates/backend-api.jsonc
{
  "$doc": {
    "baseline_rationale": "Standard backend toolchain — Node.js, TypeScript, linters, test runners, Make, Wrangler, and VCS",
    "security_note": "Docker excluded from baseline — added per-agent via override when justified",
  },
}
```

### Override `$doc`

```jsonc
// In _agents/x-backend-engineer.jsonc
{
  "$doc": {
    "agent_summary": "Backend APIs, real-time systems, Prisma, PostgreSQL, WebSockets",
    "override_rationale": "Adds Next.js, Playwright, Turbo, and Docker (ask) for full-stack backend work",
  },
}
```

### Generated Output

```jsonc
// =============================================================================
// Category: backend-api
// Baseline: Standard backend toolchain — Node.js, TypeScript, linters, ...
// ⚠ Security: Docker excluded from baseline — added per-agent via override
// =============================================================================

// --- x-backend-engineer ---
// Agent: Backend APIs, real-time systems, Prisma, PostgreSQL, WebSockets
// Override: Adds Next.js, Playwright, Turbo, and Docker (ask) for full-stack backend work
"x-backend-engineer": {
  "permission": { ... }
},
```

### Field Reference

| Context  | Field                | Required       | Purpose                                                  |
| -------- | -------------------- | -------------- | -------------------------------------------------------- |
| Template | `baseline_rationale` | Yes            | What this category baseline provides and why             |
| Template | `security_note`      | No             | Security-relevant information (elevated-privilege tools) |
| Override | `agent_summary`      | Yes            | One-line agent description                               |
| Override | `override_rationale` | When delta > 0 | Why this agent differs from baseline                     |

## Categories

| Category             | Type    | Agents | Description                                      |
| -------------------- | ------- | :----: | ------------------------------------------------ |
| `backend-api`        | Complex |   1    | Backend, database, cloud platform specialists    |
| `debugging`          | Complex |   0    | Multi-language diagnostic toolchain              |
| `frontend-ui`        | Complex |   1    | Client-side UI development                       |
| `fullstack-builders` | Complex |   0    | Comprehensive JS/TS development                  |
| `general-purpose`    | Simple  |   0    | Maximum restriction (write/edit/bash all denied) |
| `lang-go`            | Complex |   0    | Full Go toolchain                                |
| `lang-rust`          | Complex |   1    | Full Rust toolchain                              |
| `lang-specialized`   | Complex |   0    | Minimal baseline with large per-agent overrides  |
| `lang-typescript`    | Complex |   0    | Extended TypeScript toolchain                    |
| `orchestration`      | Simple  |   1    | Delegation-only coordinators (no shell)          |
| `planning`           | Simple  |   1    | Read-only analysis (no shell)                    |
| `quality-security`   | Complex |   1    | Code quality, testing, security audit            |
| `research-knowledge` | Simple  |   0    | Read-only research and knowledge generation      |
| `unrestricted`       | Simple  |   0    | Fully unrestricted (bash wildcard allow)         |

## Template Inheritance

Templates can inherit from other templates via the `$extends` field, enabling shared permission blocks without duplication.

### How It Works

- **Root templates** have `"$extends": null` — they define permissions from scratch.
- **Mixin templates** (e.g., `vcs-permissions`) hold shared permission blocks (git, jj, MCP CLI commands) that other templates inherit.
- **Leaf templates** use `$extends` to inherit one or more parent baselines before applying their own.

Resolution order: parent baselines are merged left-to-right, then the template's own baseline is applied, then per-agent overrides on top.

### Conflict & Cycle Detection

- **Conflict detection**: If two parents define the same key with different values, resolution fails with a clear error.
- **Cycle detection**: The template DAG is validated before resolution. Cycles produce an error showing the full path (e.g., `a → b → c → a`).

### Example

```jsonc
// Mixin template (vcs-permissions.jsonc):
{
  "$extends": null,  // root template — no parents
  "category": "vcs-permissions",
  "baseline": { "permission": { "git status": "deny", "jj log": "deny", ... } }
}

// Leaf template (backend-api.jsonc):
{
  "$extends": ["vcs-permissions"],
  "category": "backend-api",
  "baseline": { "permission": { "npm *": "allow", "tsc": "allow", ... } }
}
// Effective baseline = vcs-permissions entries + backend-api entries
```

The `--trace` flag on `agency resolve <agent>` shows which template contributed each permission entry.

## Development

### Prerequisites

- Rust 1.70+ (edition 2021)
- Make (optional, for convenience targets)

### Build & Test

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # All tests
cargo test <test_name>         # Single test
cargo fmt                      # Format
cargo clippy -- -D warnings    # Lint (warnings are errors)
```

### Make Targets

```bash
make build                     # cargo build
make release                   # cargo build --release
make install                   # Build release + install to ~/.local/bin
make uninstall                 # Remove installed binary
make test                      # cargo test
make generate-permissions      # Generate permissions.jsonc
make validate                  # Validate against templates + agents
make compare SOURCE=a TARGET=b # Compare two permissions files
make report                    # Generate audit-report.md
make resolve AGENT=<name>      # Resolve one agent's permissions
make check                     # fmt + clippy + test + generate + validate
make help                      # Print all targets
```

### Adding a New Agent

1. Create agent file: `_agents/<agent-name>.jsonc`
2. Set `$extends` to the appropriate template category
3. Add `$doc.agent_summary` (required)
4. Add overrides in `overrides.permission.{add,remove}` if needed
5. Add `$doc.override_rationale` if non-zero delta
6. Run `agency generate-permissions` to regenerate
7. Run `agency validate` to verify (should show N+1 agents passing)

### Adding a New Category

1. Create template file: `_templates/<category-name>.jsonc`
2. Define `baseline.permission` with the category's default toolchain
3. Add `$doc.baseline_rationale` (required)
4. Add `$doc.security_note` if security-relevant tools are included
5. Create override files for agents in this category
6. Run `agency generate-permissions` and `agency validate`

## JSON Schemas

Template and agent files use JSON Schema for validation and editor autocompletion:

- **Templates**: `https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json`
- **Agents**: `https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json`

Schema files live in `schemas/` and are referenced via the `$schema` field in every `.jsonc` file.

## Dependencies

| Crate          | Version | Purpose                                      |
| -------------- | ------- | -------------------------------------------- |
| `clap`         | 4       | CLI argument parsing (derive)                |
| `serde`        | 1       | Serialization/deserialization                |
| `serde_json`   | 1       | JSON processing                              |
| `jsonc-parser` | 0.29    | JSONC parsing (comments + trailing commas)   |
| `anyhow`       | 1       | Error handling with context                  |
| `indexmap`     | 2       | Deterministic-order maps                     |
| `bincode`      | 1       | Binary serialization for permission cache    |
| `petgraph`     | 0.6     | Template inheritance DAG and cycle detection |

## License

See [LICENSE](LICENSE) for details.
