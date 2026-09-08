mod config;
mod engine;
mod hostd;
mod model;
mod model_settings;
mod tools;

pub use config::{
    EnvironmentSecretResolver, RuntimeConfig, SecretRef, SecretRefError, SecretResolver, SecretValue,
};
pub use engine::{Clock, RuntimeEngine, RuntimeError, SystemClock};
pub use hostd::{HostExecutor, HostManagedDeployOutput, HostProcessOutput, HostSandboxOutput, HostdClient};
pub use model::{
    ChatCompletionsProvider, ModelProvider, ModelRequest, ModelResponse, ModelToolCall, ProviderError,
    ProviderErrorCategory,
};
pub use model_settings::{
    EffectiveModelSettings, EnvModelConfig, ModelSettingsSnapshot, model_settings_snapshot,
    resolve_env_model_settings, resolve_model_settings,
};
pub use tools::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput, ToolRegistry};

#[cfg(test)]
mod tests;
