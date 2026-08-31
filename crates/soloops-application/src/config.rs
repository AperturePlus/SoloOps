use std::{fmt, path::PathBuf, str::FromStr};

use anyhow::{Context, Result};
use soloops_domain::BudgetSnapshot;
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Clone, PartialEq, Eq)]
pub enum SecretRef {
    Env(String),
}

impl SecretRef {
    pub fn environment_name(&self) -> &str {
        match self {
            Self::Env(name) => name,
        }
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretRef(env:[REDACTED])")
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("env:[REDACTED]")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretRefError {
    #[error("secret reference must use the env: provider")]
    UnsupportedProvider,
    #[error("environment secret name is invalid")]
    InvalidEnvironmentName,
}

impl FromStr for SecretRef {
    type Err = SecretRefError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let Some(name) = value.strip_prefix("env:") else {
            return Err(SecretRefError::UnsupportedProvider);
        };
        let mut characters = name.chars();
        let valid_start = characters
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
        if !valid_start || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        {
            return Err(SecretRefError::InvalidEnvironmentName);
        }
        Ok(Self::Env(name.to_owned()))
    }
}

pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EnvironmentSecretResolver;

impl SecretResolver for EnvironmentSecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue> {
        let name = reference.environment_name();
        let value = std::env::var(name)
            .with_context(|| format!("configured model API key environment variable {name} is not set"))?;
        if value.is_empty() {
            anyhow::bail!("configured model API key environment variable {name} is empty");
        }
        Ok(SecretValue::new(value))
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub model_base_url: String,
    pub model_name: String,
    pub model_api_key_ref: SecretRef,
    pub prompt_cache_key_enabled: bool,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub hostd_socket: Option<PathBuf>,
    pub sandbox_enabled: bool,
    pub managed_deploy_enabled: bool,
    pub budget: BudgetSnapshot,
    pub lease_ms: i64,
    pub lease_renew_ms: u64,
}

impl RuntimeConfig {
    pub fn load() -> Result<Self> {
        let model_name = required_env("SOLOOPS_MODEL_NAME")?;
        let primary_key_ref = std::env::var("SOLOOPS_MODEL_API_KEY_REF").ok();
        let legacy_key_env = std::env::var("SOLOOPS_MODEL_API_KEY_ENV").ok();
        let (model_api_key_ref, used_legacy_key_env) =
            model_api_key_ref(primary_key_ref.as_deref(), legacy_key_env.as_deref())?;
        if used_legacy_key_env {
            eprintln!("SOLOOPS_MODEL_API_KEY_ENV is deprecated; use SOLOOPS_MODEL_API_KEY_REF=env:<name>");
        }
        let old_allowlist_is_configured =
            std::env::var("SOLOOPS_PROCESS_ALLOWLIST_JSON")
                .ok()
                .is_some_and(|value| {
                    let value = value.trim();
                    !value.is_empty() && value != "{}"
                });
        let hostd_socket = std::env::var("SOLOOPS_HOSTD_SOCKET")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        if hostd_socket.is_none() && old_allowlist_is_configured {
            anyhow::bail!(
                "SOLOOPS_PROCESS_ALLOWLIST_JSON requires SOLOOPS_HOSTD_SOCKET; local process execution was removed"
            );
        }
        let sandbox_enabled = parse_bool(&env("SOLOOPS_SANDBOX_ENABLED", "false"))?;
        if sandbox_enabled && hostd_socket.is_none() {
            anyhow::bail!("SOLOOPS_SANDBOX_ENABLED requires SOLOOPS_HOSTD_SOCKET");
        }
        let managed_deploy_enabled = parse_bool(&env("SOLOOPS_MANAGED_DEPLOY_ENABLED", "false"))?;
        if managed_deploy_enabled && hostd_socket.is_none() {
            anyhow::bail!("SOLOOPS_MANAGED_DEPLOY_ENABLED requires SOLOOPS_HOSTD_SOCKET");
        }
        let current = std::env::current_dir().context("failed to resolve current directory")?;
        let budget = BudgetSnapshot {
            max_model_turns: parsed("SOLOOPS_MAX_MODEL_TURNS", 20, 1, 200)?,
            max_tool_calls: parsed("SOLOOPS_MAX_TOOL_CALLS", 50, 1, 1000)?,
            max_input_tokens: parsed("SOLOOPS_MAX_INPUT_TOKENS", 200_000, 1, 10_000_000)?,
            max_output_tokens: parsed("SOLOOPS_MAX_OUTPUT_TOKENS", 50_000, 1, 2_000_000)?,
            max_duration_ms: parsed("SOLOOPS_MAX_RUN_DURATION_MS", 1_800_000, 1_000, 86_400_000)?,
            max_tool_duration_ms: parsed("SOLOOPS_MAX_TOOL_DURATION_MS", 600_000, 100, 600_000)?,
            max_tool_output_bytes: parsed(
                "SOLOOPS_MAX_TOOL_OUTPUT_BYTES",
                65_536usize,
                1024,
                10 * 1024 * 1024,
            )?,
            max_workspace_bytes: parsed(
                "SOLOOPS_MAX_WORKSPACE_BYTES",
                100 * 1024 * 1024,
                1024,
                10 * 1024 * 1024 * 1024,
            )?,
        };
        let lease_ms = parsed("SOLOOPS_WORKER_LEASE_MS", 30_000i64, 5_000, 300_000)?;
        let lease_renew_ms = parsed("SOLOOPS_WORKER_LEASE_RENEW_MS", 10_000u64, 1_000, 60_000)?;
        if lease_renew_ms as i64 >= lease_ms {
            anyhow::bail!("SOLOOPS_WORKER_LEASE_RENEW_MS must be less than SOLOOPS_WORKER_LEASE_MS");
        }
        Ok(Self {
            model_base_url: env("SOLOOPS_MODEL_BASE_URL", "https://api.openai.com/v1"),
            model_name,
            model_api_key_ref,
            prompt_cache_key_enabled: parse_bool(&env("SOLOOPS_PROMPT_CACHE_KEY_ENABLED", "false"))?,
            workspace_root: absolute(&current, &env("SOLOOPS_WORKSPACE_ROOT", "var/workspaces")),
            artifact_root: absolute(&current, &env("SOLOOPS_ARTIFACT_ROOT", "var/artifacts")),
            hostd_socket,
            sandbox_enabled,
            managed_deploy_enabled,
            budget,
            lease_ms,
            lease_renew_ms,
        })
    }

    pub fn api_key(&self, resolver: &dyn SecretResolver) -> Result<SecretValue> {
        resolver.resolve(&self.model_api_key_ref)
    }
}

fn model_api_key_ref(primary: Option<&str>, legacy: Option<&str>) -> Result<(SecretRef, bool)> {
    if let Some(value) = primary {
        return Ok((
            value.parse().context("SOLOOPS_MODEL_API_KEY_REF is invalid")?,
            false,
        ));
    }
    let used_legacy = legacy.is_some();
    let legacy = legacy.unwrap_or("OPENAI_API_KEY");
    Ok((
        format!("env:{legacy}")
            .parse()
            .context("SOLOOPS_MODEL_API_KEY_ENV is invalid")?,
        used_legacy,
    ))
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("invalid boolean value: {value}"),
    }
}

fn absolute(current: &std::path::Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        current.join(path)
    }
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn required_env(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} must not be empty");
    }
    Ok(value)
}

fn parsed<T>(name: &str, default: T, minimum: T, maximum: T) -> Result<T>
where
    T: std::str::FromStr + PartialOrd + Copy + fmt::Display,
    T::Err: fmt::Display,
{
    let value = match std::env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|error| anyhow::anyhow!("{name} is invalid: {error}"))?,
        Err(_) => default,
    };
    if value < minimum || value > maximum {
        anyhow::bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_environment_references() {
        assert_eq!(
            "env:OPENAI_API_KEY".parse::<SecretRef>().unwrap(),
            SecretRef::Env("OPENAI_API_KEY".into())
        );
        assert_eq!(
            "file:/secret".parse::<SecretRef>().unwrap_err(),
            SecretRefError::UnsupportedProvider
        );
        assert_eq!(
            "env:bad-name".parse::<SecretRef>().unwrap_err(),
            SecretRefError::InvalidEnvironmentName
        );
    }

    #[test]
    fn secret_debug_output_is_redacted() {
        let reference: SecretRef = "env:VERY_PRIVATE".parse().unwrap();
        let value = SecretValue::new("super-secret".into());
        let rendered = format!("{reference:?} {value:?}");
        assert!(!rendered.contains("VERY_PRIVATE"));
        assert!(!rendered.contains("super-secret"));
        assert_eq!(value.expose_secret(), "super-secret");
    }

    #[test]
    fn legacy_model_key_environment_is_compatible_for_one_phase() {
        let (reference, used_legacy) = model_api_key_ref(None, Some("LEGACY_API_KEY")).unwrap();
        assert_eq!(reference, SecretRef::Env("LEGACY_API_KEY".into()));
        assert!(used_legacy);

        let (reference, used_legacy) =
            model_api_key_ref(Some("env:NEW_API_KEY"), Some("LEGACY_API_KEY")).unwrap();
        assert_eq!(reference, SecretRef::Env("NEW_API_KEY".into()));
        assert!(!used_legacy);
    }

    #[test]
    fn missing_environment_secret_does_not_expose_a_value() {
        let reference = SecretRef::Env("SOLOOPS_TEST_VARIABLE_THAT_MUST_NOT_EXIST_8A83".into());
        let error = EnvironmentSecretResolver
            .resolve(&reference)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SOLOOPS_TEST_VARIABLE_THAT_MUST_NOT_EXIST_8A83"));
        assert!(!error.contains("super-secret"));
    }
}
