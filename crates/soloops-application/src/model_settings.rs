//! Owner-managed LLM API settings.
//!
//! The effective model endpoint is resolved the same way as SMTP delivery:
//! values saved from the WebUI (database) take precedence, otherwise the
//! `SOLOOPS_MODEL_*` environment configuration is used as the bootstrap
//! fallback. The worker re-resolves between runs and rebuilds its provider
//! whenever the effective settings change, so WebUI edits apply without a
//! restart.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use soloops_storage::{Database, ModelSettingsRecord, StorageError};
use tracing::warn;

use crate::{SecretRef, SecretResolver, SecretValue, config::EnvironmentSecretResolver};

/// Environment bootstrap for the model endpoint, read once at startup. An
/// empty model name marks the environment as unconfigured.
#[derive(Debug, Clone)]
pub struct EnvModelConfig {
    pub base_url: String,
    pub model_name: String,
    pub api_key_ref: SecretRef,
}

impl EnvModelConfig {
    pub fn from_environment() -> Result<Self> {
        let base_url = std::env::var("SOLOOPS_MODEL_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        let model_name = std::env::var("SOLOOPS_MODEL_NAME").unwrap_or_default();
        let primary_key_ref = std::env::var("SOLOOPS_MODEL_API_KEY_REF").ok();
        let legacy_key_env = std::env::var("SOLOOPS_MODEL_API_KEY_ENV").ok();
        let (api_key_ref, used_legacy_key_env) =
            model_api_key_ref(primary_key_ref.as_deref(), legacy_key_env.as_deref())?;
        if used_legacy_key_env {
            eprintln!("SOLOOPS_MODEL_API_KEY_ENV is deprecated; use SOLOOPS_MODEL_API_KEY_REF=env:<name>");
        }
        Ok(Self {
            base_url,
            model_name,
            api_key_ref,
        })
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

/// A fully resolved model endpoint. The API key is plaintext and never
/// serialized; it only lives long enough to build a provider.
pub struct EffectiveModelSettings {
    pub base_url: String,
    pub model_name: String,
    pub api_key: SecretValue,
    /// `database` or `environment`.
    pub source: &'static str,
}

impl EffectiveModelSettings {
    /// Rebuild signature: the worker compares it between runs to decide
    /// whether the provider must be rebuilt. The key is digested so the
    /// signature itself never carries the credential.
    pub fn signature(&self) -> String {
        let key_digest = Sha256::digest(self.api_key.expose_secret().as_bytes());
        format!(
            "{}|{}|{}|{:x}",
            self.source, self.base_url, self.model_name, key_digest
        )
    }

    fn from_record(record: ModelSettingsRecord) -> Self {
        Self {
            base_url: record.base_url,
            model_name: record.model_name,
            api_key: SecretValue::new(record.api_key.unwrap_or_default()),
            source: "database",
        }
    }
}

/// Effective values for the settings API; the API key is summarized as a
/// boolean instead of being exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSettingsSnapshot {
    /// `database`, `environment`, or `None` when unconfigured.
    pub source: Option<&'static str>,
    pub base_url: Option<String>,
    pub model_name: Option<String>,
    pub api_key_configured: bool,
    pub configured: bool,
}

impl ModelSettingsSnapshot {
    fn unconfigured() -> Self {
        Self {
            source: None,
            base_url: None,
            model_name: None,
            api_key_configured: false,
            configured: false,
        }
    }
}

/// Resolves the effective model settings for the given owner. Database
/// errors propagate; a broken environment fallback degrades to `None`
/// (unconfigured) instead of failing the whole request.
pub async fn resolve_model_settings(
    database: &Database,
    owner_id: &str,
    env: &EnvModelConfig,
) -> Result<Option<EffectiveModelSettings>, StorageError> {
    if let Some(record) = database.get_model_settings(owner_id).await? {
        return Ok(Some(EffectiveModelSettings::from_record(record)));
    }
    Ok(resolve_env_model_settings(env))
}

/// Resolves the effective model settings without an owner row, mirroring the
/// SMTP bootstrap fallback. `None` means unconfigured (or a broken
/// environment reference, which is logged once per call).
pub fn resolve_env_model_settings(env: &EnvModelConfig) -> Option<EffectiveModelSettings> {
    if env.model_name.trim().is_empty() {
        return None;
    }
    match EnvironmentSecretResolver.resolve(&env.api_key_ref) {
        Ok(api_key) => Some(EffectiveModelSettings {
            base_url: env.base_url.clone(),
            model_name: env.model_name.clone(),
            api_key,
            source: "environment",
        }),
        Err(error) => {
            warn!(%error, "environment model API settings are incomplete; ignoring them");
            None
        }
    }
}

/// Snapshot of the effective settings for the settings API.
pub async fn model_settings_snapshot(
    database: &Database,
    owner_id: &str,
    env: &EnvModelConfig,
) -> Result<ModelSettingsSnapshot, StorageError> {
    if let Some(record) = database.get_model_settings(owner_id).await? {
        let configured = !record.base_url.trim().is_empty() && !record.model_name.trim().is_empty();
        return Ok(ModelSettingsSnapshot {
            source: Some("database"),
            base_url: Some(record.base_url),
            model_name: Some(record.model_name),
            api_key_configured: record.api_key.as_deref().is_some_and(|key| !key.is_empty()),
            configured,
        });
    }
    if env.model_name.trim().is_empty() {
        return Ok(ModelSettingsSnapshot::unconfigured());
    }
    let api_key_configured = EnvironmentSecretResolver
        .resolve(&env.api_key_ref)
        .map(|key| !key.expose_secret().is_empty())
        .unwrap_or(false);
    Ok(ModelSettingsSnapshot {
        source: Some("environment"),
        base_url: Some(env.base_url.clone()),
        model_name: Some(env.model_name.clone()),
        api_key_configured,
        configured: api_key_configured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soloops_storage::now_ms;

    async fn migrated_database() -> Database {
        let database = Database::connect(":memory:").await.unwrap();
        database.migrate().await.unwrap();
        database
    }

    fn stored_record(owner_id: &str, api_key: Option<&str>) -> ModelSettingsRecord {
        ModelSettingsRecord {
            owner_id: owner_id.to_owned(),
            base_url: "https://models.example.com/v1".into(),
            model_name: "example-model".into(),
            api_key: api_key.map(str::to_owned),
            updated_at: now_ms(),
        }
    }

    fn env_config(model_name: &str) -> EnvModelConfig {
        EnvModelConfig {
            base_url: "https://env.example.com/v1".into(),
            model_name: model_name.into(),
            // A variable no test ever sets, so the environment fallback is
            // deterministically unresolvable in the parallel async tests.
            api_key_ref: "env:SOLOOPS_MODEL_TEST_UNSET_KEY_7C31".parse().unwrap(),
        }
    }

    fn resolvable_env_config(model_name: &str) -> EnvModelConfig {
        EnvModelConfig {
            base_url: "https://env.example.com/v1".into(),
            model_name: model_name.into(),
            api_key_ref: "env:SOLOOPS_MODEL_TEST_KEY_9F2A".parse().unwrap(),
        }
    }

    #[tokio::test]
    async fn database_settings_take_precedence_over_the_environment() {
        let database = migrated_database().await;
        let owner = database.create_owner("owner", "hash").await.unwrap();

        // Unset key variable: the environment fallback is unusable.
        assert!(
            resolve_model_settings(&database, &owner.id, &env_config("env-model"))
                .await
                .unwrap()
                .is_none()
        );
        let snapshot = model_settings_snapshot(&database, &owner.id, &env_config("env-model"))
            .await
            .unwrap();
        assert_eq!(snapshot.source, Some("environment"));
        assert_eq!(snapshot.model_name.as_deref(), Some("env-model"));
        assert!(!snapshot.api_key_configured);
        assert!(!snapshot.configured);

        database
            .upsert_model_settings(&stored_record(&owner.id, Some("secret-key")))
            .await
            .unwrap();
        let effective = resolve_model_settings(&database, &owner.id, &env_config("env-model"))
            .await
            .unwrap()
            .expect("database settings must win");
        assert_eq!(effective.source, "database");
        assert_eq!(effective.base_url, "https://models.example.com/v1");
        assert_eq!(effective.model_name, "example-model");
        assert_eq!(effective.api_key.expose_secret(), "secret-key");

        let snapshot = model_settings_snapshot(&database, &owner.id, &env_config("env-model"))
            .await
            .unwrap();
        assert_eq!(snapshot.source, Some("database"));
        assert!(snapshot.api_key_configured);
        assert!(snapshot.configured);

        // Rotating the key changes the signature; the endpoint does not.
        let before = effective.signature();
        database
            .upsert_model_settings(&stored_record(&owner.id, Some("rotated-key")))
            .await
            .unwrap();
        let rotated = resolve_model_settings(&database, &owner.id, &env_config("env-model"))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(before, rotated.signature());
        assert_eq!(before, {
            let settings = EffectiveModelSettings {
                base_url: "https://models.example.com/v1".into(),
                model_name: "example-model".into(),
                api_key: SecretValue::new("secret-key".into()),
                source: "database",
            };
            settings.signature()
        });

        assert!(database.delete_model_settings(&owner.id).await.unwrap());
        assert!(
            resolve_model_settings(&database, &owner.id, &env_config("env-model"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn stored_settings_without_a_key_stay_usable() {
        let database = migrated_database().await;
        let owner = database.create_owner("owner", "hash").await.unwrap();
        database
            .upsert_model_settings(&stored_record(&owner.id, None))
            .await
            .unwrap();
        let effective = resolve_model_settings(&database, &owner.id, &env_config("env-model"))
            .await
            .unwrap()
            .expect("a keyless row still configures the endpoint");
        assert_eq!(effective.api_key.expose_secret(), "");
        let snapshot = model_settings_snapshot(&database, &owner.id, &env_config("env-model"))
            .await
            .unwrap();
        assert!(snapshot.configured);
        assert!(!snapshot.api_key_configured);
    }

    #[test]
    fn environment_without_a_model_name_is_unconfigured() {
        // SAFETY: a unique test-only variable name never collides with a
        // parallel test and is removed before the assertions below run.
        unsafe { std::env::set_var("SOLOOPS_MODEL_TEST_KEY_9F2A", "env-secret") };
        assert!(
            resolve_env_model_settings(&resolvable_env_config("")).is_none(),
            "an empty model name must disable the environment fallback"
        );
        let effective = resolve_env_model_settings(&resolvable_env_config("env-model"))
            .expect("a resolvable key must enable the environment fallback");
        assert_eq!(effective.source, "environment");
        assert_eq!(effective.base_url, "https://env.example.com/v1");
        assert_eq!(effective.api_key.expose_secret(), "env-secret");
        unsafe { std::env::remove_var("SOLOOPS_MODEL_TEST_KEY_9F2A") };
    }

    #[test]
    fn a_broken_environment_reference_degrades_to_unconfigured() {
        assert!(
            resolve_env_model_settings(&env_config("env-model")).is_none(),
            "a missing key variable must degrade to unconfigured instead of failing"
        );
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
    fn a_missing_primary_reference_is_rejected() {
        assert!(model_api_key_ref(Some("not-env"), None).is_err());
        assert!(model_api_key_ref(Some("env:bad-name"), None).is_err());
    }
}
