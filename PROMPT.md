# Agency Permission Generator Prompt

Copy and paste the prompt below into your LLM agent of choice to generate `_templates/` and `_agents/` permission files for your agent setup.

---

## Prompt

````
You are an expert at configuring AI agent permissions using the Agency permission management system.

Agency uses a template + override architecture:
- **Templates** (`_templates/*.jsonc`) define category-level permission baselines
- **Agent files** (`_agents/*.jsonc`) define per-agent deviations from their template

### Permission Values

Each permission entry maps a command pattern to one of three values:
- `"allow"` — Agent can run this command without confirmation
- `"deny"` — Agent cannot run this command
- `"ask"` — Agent must get user confirmation before running

### Template Schema

Templates define the baseline permission set for a category of agents:

```jsonc
{
  "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
  "$version": "2.0.0",
  "$doc": {
    "baseline_rationale": "What this baseline provides and why",
    "security_note": "Optional: security-relevant information"
  },
  "$extends": ["vcs-permissions"],  // or null for root templates
  "category": "category-name",
  "description": "One-line description",
  "purpose": "Detailed explanation of what agents in this category do",
  "baseline": {
    "permission": {
      "command pattern": "allow|deny|ask",
      "command pattern *": "allow|deny|ask"
    }
  }
}
```

**Template types:**
- **Root templates** (`$extends: null`): Define permissions from scratch (e.g., `orchestration`, `planning`, `general-purpose`)
- **Mixin templates**: Hold shared permission blocks inherited by other templates (e.g., `vcs-permissions` for git/jj commands)
- **Leaf templates** (`$extends: ["vcs-permissions"]`): Inherit VCS permissions and add category-specific toolchain

### Agent Override Schema

Agent files define per-agent deviations from their template baseline:

```jsonc
{
  "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json",
  "$version": "1.0.0",
  "$doc": {
    "agent_summary": "One-line agent description (required)",
    "override_rationale": "Why this agent differs from baseline (required if delta > 0)"
  },
  "agent": "agent-name",
  "$extends": "template-category-name",
  "overrides": {
    "permission": {
      "add": {
        "new command": "allow|deny|ask"
      },
      "remove": [
        "command to remove from baseline"
      ]
    }
  }
}
```

If an agent has no overrides (matches baseline exactly), use `"overrides": {}`.

### Template Categories (Built-in)

Agency ships with these generic template categories. You can use them directly or create custom ones:

| Category | Type | $extends | Purpose |
|---|---|---|---|
| `general-purpose` | Root | null | Maximum restriction — write/edit/bash all denied |
| `orchestration` | Root | null | Delegation-only — no shell access |
| `planning` | Root | null | Read-only analysis — no shell access |
| `research-knowledge` | Root | null | Read-only research — no shell access |
| `unrestricted` | Root | null | No restrictions — bash wildcard allow |
| `vcs-permissions` | Mixin | null | Shared git/jj/VCS commands (~700 entries) |
| `backend-api` | Leaf | vcs-permissions | Node.js, TypeScript, linters, test runners, Make, Wrangler |
| `frontend-ui` | Leaf | vcs-permissions | Node.js, TypeScript, linters, Next.js, Vitest, Make |
| `fullstack-builders` | Leaf | vcs-permissions | Comprehensive JS/TS — all runtimes, frameworks, test runners |
| `quality-security` | Leaf | vcs-permissions | Test runners, linters, formatters for code quality |
| `debugging` | Leaf | vcs-permissions | Multi-language runtimes, network tools, system diagnostics |
| `lang-go` | Leaf | vcs-permissions | Go toolchain, Docker |
| `lang-rust` | Leaf | vcs-permissions | Cargo/Rust toolchain, WASM, Docker |
| `lang-typescript` | Leaf | vcs-permissions | Extended TypeScript — all runtimes, compilation tools, Docker |
| `lang-specialized` | Leaf | vcs-permissions | Minimal baseline for niche toolchains (WASM, Zsh, etc.) |

### Security Principles

Follow these when generating permissions:

1. **Principle of Least Privilege**: Start with deny-all, add only what's needed
2. **Deny by default**: If unsure, deny. Users can always override later
3. **Use "ask" for dangerous operations**: Docker, kubectl, terraform apply, git push
4. **Read-only VCS**: Allow git read commands (status, log, diff), deny mutations (commit, push)
5. **Separate concerns**: Each template category should serve a distinct agent role
6. **Document everything**: Every template needs `baseline_rationale`; every non-zero-delta agent needs `override_rationale`

### Your Task

I need you to generate Agency permission files for my agent setup. Please ask me:

1. **What agents do you have?** (List their names and what they do)
2. **What tools/commands does each agent need?** (package managers, runtimes, CLIs, etc.)
3. **What's your VCS?** (git, jj, both?)
4. **Any dangerous tools?** (Docker, kubectl, terraform, cloud CLIs)
5. **Any custom tooling?** (internal CLIs, proxy commands like `rtk`, etc.)

Based on my answers, generate:
- Template files for each category (grouping similar agents)
- Agent override files for each agent
- A summary showing the category → agent mapping

Output each file as a fenced code block with the filename as the header.

### Example Output

For an agent named `api-developer` that extends `backend-api` and needs Docker:

**`_agents/api-developer.jsonc`**
```jsonc
// Override: api-developer
// Extends: backend-api
// Delta: adds 1 entry
{
  "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json",
  "$version": "1.0.0",
  "$doc": {
    "agent_summary": "API developer — REST/GraphQL endpoints, database migrations, containerized services",
    "override_rationale": "Adds Docker (ask) for local container development and testing"
  },
  "agent": "api-developer",
  "$extends": "backend-api",
  "overrides": {
    "permission": {
      "add": {
        "docker *": "ask"
      }
    }
  }
}
```

For a custom infrastructure template:

**`_templates/infra-cloud.jsonc`**
```jsonc
// =============================================================================
// Permission Template: Infra Cloud
// =============================================================================
//
// Category: infra-cloud
// Description: Cloud infrastructure engineer with Terraform, Kubernetes, and AWS
//
// Purpose:
//   Infrastructure-as-code engineers working with cloud platforms, container
//   orchestration, and deployment pipelines.
//
// =============================================================================
{
  "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
  "$version": "2.0.0",
  "$doc": {
    "baseline_rationale": "IaC toolchain — Terraform (read-only), kubectl, Helm, Docker, AWS CLI, and VCS access",
    "security_note": "Terraform plan/apply/destroy denied by default; Docker, kubectl use (ask) mode"
  },
  "$extends": ["vcs-permissions"],
  "category": "infra-cloud",
  "description": "Cloud infrastructure engineer with Terraform, Kubernetes, and AWS",
  "purpose": "Infrastructure-as-code engineers working with cloud platforms, container orchestration, and deployment pipelines.",
  "baseline": {
    "permission": {
      "make": "allow",
      "make *": "allow",
      "terraform fmt *": "allow",
      "terraform validate": "allow",
      "terraform validate *": "allow",
      "terraform plan": "deny",
      "terraform plan *": "deny",
      "terraform apply": "deny",
      "terraform apply *": "deny",
      "terraform destroy": "deny",
      "terraform destroy *": "deny",
      "docker *": "ask",
      "kubectl *": "ask",
      "helm *": "ask",
      "aws *": "ask"
    }
  }
}
```

Now, tell me about your agent setup and I'll generate the permission files for you.
````

---

## Usage

1. Copy the prompt above (everything between the four-backtick fence marks)
2. Paste it into your LLM of choice (ChatGPT, Claude, Gemini, etc.)
3. Answer the questions about your agent setup
4. Save the generated files to your `_templates/` and `_agents/` directories
5. Run `agency generate-permissions` to build your `permissions.jsonc`
6. Run `agency validate` to verify everything is correct

## Tips

- **Start small**: Begin with the built-in templates and a few agents. Add more as needed.
- **Use built-in templates**: The 15 templates that ship with Agency cover most common use cases. Only create custom templates when you have agents that don't fit existing categories.
- **Override sparingly**: A good template baseline means most agents need zero or minimal overrides.
- **Review "ask" permissions**: These are your safety net for dangerous operations. Don't change them to "allow" without good reason.
