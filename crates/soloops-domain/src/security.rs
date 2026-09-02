use serde::{Deserialize, Serialize};

/// Role an authorized_keys file plays for the local sshd.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SshKeyFileRole {
    /// Per-user file, e.g. `~/.ssh/authorized_keys`.
    User,
    /// Windows OpenSSH file shared by the Administrators group,
    /// e.g. `C:\ProgramData\ssh\administrators_authorized_keys`.
    Administrators,
}

/// One parsed line of an authorized_keys file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshAuthorizedKeyEntry {
    /// 1-based line number inside the file.
    pub line: u32,
    /// Declared key type, e.g. `ssh-ed25519`.
    pub key_type: Option<String>,
    /// Significant bits of the key (3072 for RSA, 256 for ed25519).
    pub key_bits: Option<u32>,
    /// OpenSSH-style `SHA256:<base64>` fingerprint of the raw key blob.
    pub fingerprint: Option<String>,
    /// Trailing comment of the entry, often `user@hostname`.
    pub comment: Option<String>,
    /// Machine hint extracted from the comment (hostname part of `user@hostname`).
    pub machine: Option<String>,
    /// Raw option tokens, e.g. `no-pty`, `command="..."`.
    pub options: Vec<String>,
    /// `from=` source patterns when the key is host-restricted.
    pub from_patterns: Option<Vec<String>>,
    /// Forced `command=` when the key only runs a single command.
    pub forced_command: Option<String>,
    /// Whether sshd could accept this line as a public key.
    pub valid: bool,
    pub error: Option<String>,
}

/// One scanned authorized_keys file with its parsed entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshAuthorizedKeysFile {
    pub path: String,
    pub role: SshKeyFileRole,
    pub exists: bool,
    pub entries: Vec<SshAuthorizedKeyEntry>,
    pub error: Option<String>,
}

/// Aggregated view of all valid keys attributed to one inferred machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshMachineSummary {
    /// Inferred machine name; null when the key comment gives no hint.
    pub name: Option<String>,
    pub key_count: u32,
}

/// Read-only audit of the local host's SSH authorized_keys files,
/// answering which machines hold a key that may connect without a password.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshAccessReport {
    pub scanned_at: i64,
    /// OS account the per-user file belongs to, when known.
    pub user: Option<String>,
    pub home_dir: Option<String>,
    pub platform: String,
    pub total_keys: u32,
    pub invalid_lines: u32,
    pub machines: Vec<SshMachineSummary>,
    pub files: Vec<SshAuthorizedKeysFile>,
}
