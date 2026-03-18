use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::Deserialize;

const DEFAULT_TEMPLATES_DIR: &str = "_templates";
const DEFAULT_AGENTS_DIR: &str = "_agents";
const DEFAULT_GROUPS_FILE: &str = "groups.jsonc";
const CONFIG_FILE_NAME: &str = "agency.jsonc";

/// Optional config file schema (`agency.jsonc`).
#[derive(Debug, Deserialize, Default)]
struct ConfigFile {
    #[serde(default)]
    templates_dir: Option<String>,
    #[serde(default)]
    agents_dir: Option<String>,
    #[serde(default)]
    groups_file: Option<String>,
}

/// Resolved project configuration.
///
/// Priority: CLI flag → agency.jsonc config file → built-in default.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub base_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub agents_dir: PathBuf,
    pub groups: IndexMap<String, Vec<String>>,
}

impl ProjectConfig {
    /// Resolve the project configuration by merging:
    /// 1. CLI flags (highest priority)
    /// 2. `agency.jsonc` config file
    /// 3. Built-in defaults
    pub fn resolve(
        base_dir: PathBuf,
        cli_templates: Option<PathBuf>,
        cli_agents: Option<PathBuf>,
        cli_groups_file: Option<PathBuf>,
    ) -> Result<Self> {
        let config_file = load_config_file(&base_dir);
        let cfg_templates = config_file.templates_dir;
        let cfg_agents = config_file.agents_dir;

        let templates_dir = cli_templates.unwrap_or_else(|| {
            let dir_name = cfg_templates.unwrap_or_else(|| DEFAULT_TEMPLATES_DIR.to_string());
            base_dir.join(dir_name)
        });

        let agents_dir = cli_agents.unwrap_or_else(|| {
            let dir_name = cfg_agents.unwrap_or_else(|| DEFAULT_AGENTS_DIR.to_string());
            base_dir.join(dir_name)
        });

        let groups = load_groups(&base_dir, cli_groups_file, config_file.groups_file)?;

        Ok(Self {
            base_dir,
            templates_dir,
            agents_dir,
            groups,
        })
    }

    /// Return the last path component of `templates_dir` for display.
    pub fn templates_display(&self) -> &str {
        self.templates_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(DEFAULT_TEMPLATES_DIR)
    }

    /// Return the last path component of `agents_dir` for display.
    pub fn agents_display(&self) -> &str {
        self.agents_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(DEFAULT_AGENTS_DIR)
    }

    /// Validate that required directories exist.
    /// Returns a descriptive error with hints if a directory is missing.
    pub fn validate_dirs(&self) -> Result<()> {
        if !self.templates_dir.exists() {
            anyhow::bail!(
                "Templates directory not found: {}\n  \
                 Hint: Create the directory or specify a custom path with --templates-dir",
                self.templates_dir.display()
            );
        }
        if !self.agents_dir.exists() {
            anyhow::bail!(
                "Agents directory not found: {}\n  \
                 Hint: Create the directory or specify a custom path with --agents-dir",
                self.agents_dir.display()
            );
        }
        Ok(())
    }
}

/// Load and parse the optional `agency.jsonc` config file.
///
/// Returns defaults if the file does not exist.
fn load_config_file(base_dir: &Path) -> ConfigFile {
    let path = base_dir.join(CONFIG_FILE_NAME);
    if !path.exists() {
        return ConfigFile::default();
    }

    crate::jsonc::parse_file::<ConfigFile>(&path)
        .with_context(|| format!("Parsing config file: {}", path.display()))
        .unwrap_or_default()
}

/// Load command group classification.
///
/// Priority: CLI `--groups-file` → `agency.jsonc` `groups_file` →
/// `groups.jsonc` at project root → built-in `default_groups()`.
fn load_groups(
    base_dir: &Path,
    cli_groups_file: Option<PathBuf>,
    cfg_groups_file: Option<String>,
) -> Result<IndexMap<String, Vec<String>>> {
    // 1. CLI flag
    if let Some(ref path) = cli_groups_file {
        return crate::jsonc::parse_file::<IndexMap<String, Vec<String>>>(path)
            .with_context(|| format!("Loading groups file: {}", path.display()));
    }

    // 2. agency.jsonc groups_file field
    if let Some(ref relative) = cfg_groups_file {
        let path = base_dir.join(relative);
        if path.exists() {
            return crate::jsonc::parse_file::<IndexMap<String, Vec<String>>>(&path)
                .with_context(|| format!("Loading groups file: {}", path.display()));
        }
    }

    // 3. Default groups.jsonc at project root
    let default_path = base_dir.join(DEFAULT_GROUPS_FILE);
    if default_path.exists() {
        return crate::jsonc::parse_file::<IndexMap<String, Vec<String>>>(&default_path)
            .with_context(|| format!("Loading groups file: {}", default_path.display()));
    }

    // 4. Built-in defaults
    Ok(default_groups())
}

/// Built-in default command group classification.
///
/// Returns the same mapping that was previously hardcoded in `generator.rs`.
/// Key insertion order defines display order.
pub fn default_groups() -> IndexMap<String, Vec<String>> {
    let mut groups = IndexMap::new();

    groups.insert(
        "Package Managers & Runtimes".to_string(),
        vec![
            "npm", "npx", "pnpm", "bun", "bunx", "node", "deno", "python", "python3", "brew",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    groups.insert(
        "TypeScript & Compilation".to_string(),
        vec!["tsc", "swc", "esbuild", "rollup", "vite", "tsx", "ts-node"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Linting & Formatting".to_string(),
        vec!["biome", "eslint", "prettier"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Frameworks".to_string(),
        vec!["next", "hono", "nest"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Testing".to_string(),
        vec!["vitest", "playwright", "jest"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Build Systems".to_string(),
        vec!["make", "turbo", "cmake"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Cloud & Platform Tools".to_string(),
        vec!["wrangler", "ray"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Docker".to_string(),
        vec!["docker"].into_iter().map(String::from).collect(),
    );
    groups.insert(
        "Shell Tools".to_string(),
        vec!["zsh", "shellcheck", "shfmt", "chmod"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "macOS System Tools".to_string(),
        vec![
            "defaults",
            "launchctl",
            "open",
            "osascript",
            "pbcopy",
            "pbpaste",
            "sw_vers",
            "sysctl",
            "system_profiler",
            "plutil",
            "xcode-select",
            "xcrun",
            "otool",
            "codesign",
            "csrutil",
            "spctl",
            "security",
            "diskutil",
            "mdls",
            "mdfind",
            "pmset",
            "powermetrics",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    groups.insert(
        "MCP CLI".to_string(),
        vec!["mcp-cli", "$mcp_cli_bin", "mcp_cli_bin"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Network Debugging".to_string(),
        vec![
            "curl",
            "wget",
            "nc",
            "nslookup",
            "dig",
            "ping",
            "traceroute",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    groups.insert(
        "System & Process".to_string(),
        vec!["ps", "top", "lsof", "netstat"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Kubernetes & Helm".to_string(),
        vec!["kubectl", "helm"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Terraform".to_string(),
        vec!["terraform", "terragrunt"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Infrastructure".to_string(),
        vec!["aws"].into_iter().map(String::from).collect(),
    );
    groups.insert(
        "Go Toolchain".to_string(),
        vec!["go", "golangci-lint", "staticcheck", "dlv", "protoc", "buf"]
            .into_iter()
            .map(String::from)
            .collect(),
    );
    groups.insert(
        "Rust Toolchain".to_string(),
        vec![
            "cargo",
            "rustc",
            "rustfmt",
            "clippy-driver",
            "rustup",
            "rust-analyzer",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );
    groups.insert(
        "WASM Toolchain".to_string(),
        vec![
            "wasm-pack",
            "wasm-opt",
            "wasm-tools",
            "wasmtime",
            "wasmer",
            "wasm2wat",
            "wat2wasm",
            "tinygo",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    );

    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_when_no_config_file() {
        let base = PathBuf::from("/tmp/nonexistent-project-dir");
        let config = ProjectConfig::resolve(base.clone(), None, None, None).unwrap();

        assert_eq!(config.base_dir, base);
        assert_eq!(config.templates_dir, base.join("_templates"));
        assert_eq!(config.agents_dir, base.join("_agents"));
    }

    #[test]
    fn test_cli_overrides_take_priority() {
        let base = PathBuf::from("/tmp/some-project");
        let cli_tpl = Some(PathBuf::from("/custom/templates"));
        let cli_agents = Some(PathBuf::from("/custom/agents"));

        let config = ProjectConfig::resolve(base.clone(), cli_tpl, cli_agents, None).unwrap();

        assert_eq!(config.templates_dir, PathBuf::from("/custom/templates"));
        assert_eq!(config.agents_dir, PathBuf::from("/custom/agents"));
    }

    #[test]
    fn test_templates_display() {
        let base = PathBuf::from("/tmp/project");
        let config = ProjectConfig::resolve(base, None, None, None).unwrap();
        assert_eq!(config.templates_display(), "_templates");
        assert_eq!(config.agents_display(), "_agents");
    }

    #[test]
    fn test_display_with_custom_paths() {
        let config = ProjectConfig {
            base_dir: PathBuf::from("/tmp"),
            templates_dir: PathBuf::from("/tmp/_my_templates"),
            agents_dir: PathBuf::from("/tmp/_agents"),
            groups: default_groups(),
        };
        assert_eq!(config.templates_display(), "_my_templates");
        assert_eq!(config.agents_display(), "_agents");
    }

    #[test]
    fn test_missing_config_file_returns_defaults() {
        let config_file = load_config_file(Path::new("/tmp/nonexistent-dir-12345"));
        assert!(config_file.templates_dir.is_none());
        assert!(config_file.agents_dir.is_none());
    }

    #[test]
    fn test_validate_dirs_missing_templates() {
        let config = ProjectConfig {
            base_dir: PathBuf::from("/tmp/nonexistent-agency-test"),
            templates_dir: PathBuf::from("/tmp/nonexistent-agency-test/_templates"),
            agents_dir: PathBuf::from("/tmp/nonexistent-agency-test/_agents"),
            groups: default_groups(),
        };
        let result = config.validate_dirs();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Templates directory not found"),
            "got: {}",
            err
        );
        assert!(err.contains("--templates-dir"), "got: {}", err);
    }

    #[test]
    fn test_validate_dirs_missing_agents_only() {
        // Create a temp dir with _templates but no _agents
        let tmp = std::env::temp_dir().join("agency-test-validate-dirs");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("_templates")).unwrap();

        let config = ProjectConfig {
            base_dir: tmp.clone(),
            templates_dir: tmp.join("_templates"),
            agents_dir: tmp.join("_agents"),
            groups: default_groups(),
        };
        let result = config.validate_dirs();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Agents directory not found"), "got: {}", err);
        assert!(err.contains("--agents-dir"), "got: {}", err);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_validate_dirs_both_exist() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = ProjectConfig::resolve(base, None, None, None).unwrap();
        assert!(config.validate_dirs().is_ok());
    }

    #[test]
    fn test_default_groups_has_expected_keys() {
        let groups = default_groups();
        assert!(groups.contains_key("Docker"), "should contain Docker group");
        assert!(
            groups.contains_key("Go Toolchain"),
            "should contain Go Toolchain group"
        );
        assert!(
            groups.contains_key("Package Managers & Runtimes"),
            "should contain Package Managers & Runtimes group"
        );
        assert!(
            groups.contains_key("Rust Toolchain"),
            "should contain Rust Toolchain group"
        );
        assert!(
            groups.contains_key("WASM Toolchain"),
            "should contain WASM Toolchain group"
        );
    }

    #[test]
    fn test_default_groups_order_preserved() {
        let groups = default_groups();
        let keys: Vec<&String> = groups.keys().collect();
        assert_eq!(keys[0], "Package Managers & Runtimes");
        assert_eq!(keys[1], "TypeScript & Compilation");
        assert_eq!(keys[2], "Linting & Formatting");
        assert_eq!(keys[3], "Frameworks");
        assert_eq!(keys[4], "Testing");
        // Last key should be WASM Toolchain
        assert_eq!(keys[keys.len() - 1], "WASM Toolchain");
    }

    #[test]
    fn test_groups_fallback_to_defaults_when_no_file() {
        let base = PathBuf::from("/tmp/nonexistent-project-dir");
        let config = ProjectConfig::resolve(base, None, None, None).unwrap();
        // Should have loaded built-in defaults since no groups.jsonc exists
        assert!(
            config.groups.contains_key("Docker"),
            "fallback groups should contain Docker"
        );
    }
}
