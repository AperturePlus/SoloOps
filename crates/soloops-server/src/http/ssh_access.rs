//! Local SSH authorized_keys audit.
//!
//! Reads the OpenSSH `authorized_keys` files that govern passwordless login
//! to this host, parses every entry, and groups the results by the machine
//! hinted at in each key comment (`user@hostname`).

use std::{collections::HashMap, path::Path, path::PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use soloops_domain::{
    SshAccessReport, SshAuthorizedKeyEntry, SshAuthorizedKeysFile, SshKeyFileRole, SshMachineSummary,
};
use soloops_storage::now_ms;

/// Windows OpenSSH stores keys for the Administrators group system-wide.
#[cfg(windows)]
const ADMINISTRATORS_AUTHORIZED_KEYS: &str = r"C:\ProgramData\ssh\administrators_authorized_keys";

/// Scan the local host's authorized_keys files for the current environment.
pub(super) fn scan_ssh_access() -> SshAccessReport {
    scan_with_home(home_dir().as_deref(), current_user(), std::env::consts::OS)
}

/// Environment-independent core of [`scan_ssh_access`], kept pure for tests.
pub(super) fn scan_with_home(home: Option<&Path>, user: Option<String>, platform: &str) -> SshAccessReport {
    let mut files = Vec::new();
    match home {
        Some(home) => files.push(read_file(
            home.join(".ssh").join("authorized_keys"),
            SshKeyFileRole::User,
        )),
        None => files.push(SshAuthorizedKeysFile {
            path: "~/.ssh/authorized_keys".to_owned(),
            role: SshKeyFileRole::User,
            exists: false,
            entries: Vec::new(),
            error: Some("home directory could not be determined".to_owned()),
        }),
    }
    #[cfg(windows)]
    files.push(read_file(
        PathBuf::from(ADMINISTRATORS_AUTHORIZED_KEYS),
        SshKeyFileRole::Administrators,
    ));

    let total_keys = files
        .iter()
        .map(|file| file.entries.iter().filter(|entry| entry.valid).count())
        .sum::<usize>() as u32;
    let invalid_lines = files
        .iter()
        .map(|file| file.entries.iter().filter(|entry| !entry.valid).count())
        .sum::<usize>() as u32;

    SshAccessReport {
        scanned_at: now_ms(),
        user,
        home_dir: home.map(|home| home.to_string_lossy().into_owned()),
        platform: platform.to_owned(),
        total_keys,
        invalid_lines,
        machines: summarize_machines(&files),
        files,
    }
}

fn read_file(path: PathBuf, role: SshKeyFileRole) -> SshAuthorizedKeysFile {
    let display = path.to_string_lossy().into_owned();
    if !path.is_file() {
        return SshAuthorizedKeysFile {
            path: display,
            role,
            exists: false,
            entries: Vec::new(),
            error: None,
        };
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => SshAuthorizedKeysFile {
            path: display,
            role,
            exists: true,
            entries: parse_authorized_keys(&content),
            error: None,
        },
        Err(error) => SshAuthorizedKeysFile {
            path: display,
            role,
            exists: true,
            entries: Vec::new(),
            error: Some(format!("failed to read the file: {error}")),
        },
    }
}

/// Parse authorized_keys content into entries. Blank lines and `#` comments
/// are skipped; every other line yields an entry (invalid lines included).
pub(super) fn parse_authorized_keys(content: &str) -> Vec<SshAuthorizedKeyEntry> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| parse_line(line, index as u32 + 1))
        .collect()
}

/// Parse one authorized_keys line. Returns `None` for blank/comment lines.
fn parse_line(raw: &str, line_number: u32) -> Option<SshAuthorizedKeyEntry> {
    let trimmed = raw.trim_end();
    if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
        return None;
    }
    let invalid = |key_type: Option<String>, message: &str| SshAuthorizedKeyEntry {
        line: line_number,
        key_type,
        key_bits: None,
        fingerprint: None,
        comment: None,
        machine: None,
        options: Vec::new(),
        from_patterns: None,
        forced_command: None,
        valid: false,
        error: Some(message.to_owned()),
    };

    let (options_raw, key_type, rest) = match split_key_start(trimmed) {
        Some(split) => split,
        None => {
            return Some(invalid(
                None,
                "line does not contain a recognizable public key type",
            ));
        }
    };
    let (key_data, comment) = match rest.split_once(char::is_whitespace) {
        Some((data, comment)) => (data, Some(comment.trim()).filter(|value| !value.is_empty())),
        None => (rest.trim(), None),
    };
    let comment = comment.map(str::to_owned);

    let blob = match decode_key_data(key_data) {
        Some(blob) => blob,
        None => {
            return Some(invalid(Some(key_type.to_owned()), "key data is not valid base64"));
        }
    };
    let fingerprint = format!("SHA256:{}", STANDARD.encode(Sha256::digest(&blob)));
    let fields = parse_ssh_strings(&blob);
    if fields.first() != Some(&key_type.as_bytes()) {
        return Some(invalid(
            Some(key_type.to_owned()),
            "key blob does not match the declared key type",
        ));
    }

    let options = parse_options(options_raw);
    let from_patterns = options
        .iter()
        .find_map(|option| option_value(option, "from"))
        .map(|value| split_patterns(&value));
    let forced_command = options.iter().find_map(|option| option_value(option, "command"));

    Some(SshAuthorizedKeyEntry {
        line: line_number,
        key_type: Some(key_type.to_owned()),
        key_bits: key_bits(key_type, &fields),
        fingerprint: Some(fingerprint),
        machine: machine_hint(comment.as_deref()),
        comment,
        options,
        from_patterns,
        forced_command,
        valid: true,
        error: None,
    })
}

/// Locate the key type token in an authorized_keys line, honoring quoted
/// option values. Returns `(options_raw, key_type, rest_after_type)`.
/// Token separators outside quotes are whitespace and commas, matching sshd's
/// tolerant parsing of `option,value` lists written without spaces.
fn split_key_start(line: &str) -> Option<(&str, &str, &str)> {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut start = 0usize;
    for (index, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if in_quotes => escaped = true,
            b'"' => in_quotes = !in_quotes,
            b' ' | b'\t' | b',' if !in_quotes => {
                let token = &line[start..index];
                if is_key_type_token(token) {
                    let options_raw = if start == 0 { "" } else { line[..start].trim_end() };
                    let rest = line[index + 1..].trim_start();
                    return Some((options_raw, token, rest));
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    None
}

fn is_key_type_token(token: &str) -> bool {
    token.starts_with("ssh-") || token.starts_with("ecdsa-") || token.starts_with("sk-")
}

fn decode_key_data(data: &str) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }
    STANDARD
        .decode(data)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(data))
        .ok()
}

/// Split an options string on top-level commas, honoring quoted values.
fn parse_options(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut options = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for character in raw.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if in_quotes => {
                current.push(character);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                current.push(character);
            }
            ',' if !in_quotes => {
                options.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    options.push(current.trim().to_owned());
    options.retain(|option| !option.is_empty());
    options
}

/// Extract the unquoted value of `name="value"` from one option token.
fn option_value(option: &str, name: &str) -> Option<String> {
    let rest = option.strip_prefix(name)?.strip_prefix('=')?;
    let value = rest.trim();
    let value = value.strip_prefix('"').unwrap_or(value);
    let value = value.strip_suffix('"').unwrap_or(value);
    Some(value.to_owned())
}

fn split_patterns(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Machine hint from a key comment: the hostname part of the first
/// `user@hostname` token, lowercased. Returns `None` when the comment gives
/// no usable hint.
fn machine_hint(comment: Option<&str>) -> Option<String> {
    let first = comment?.split_whitespace().next()?;
    let (_, host) = first.rsplit_once('@')?;
    let host = host.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Split an SSH key blob into its wire-format string fields.
fn parse_ssh_strings(blob: &[u8]) -> Vec<&[u8]> {
    let mut fields = Vec::new();
    let mut rest = blob;
    while rest.len() >= 4 {
        let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if rest.len() < 4 + length {
            break;
        }
        fields.push(&rest[4..4 + length]);
        rest = &rest[4 + length..];
    }
    fields
}

/// Significant bits of an SSH mpint (leading zero byte stripped by the
/// encoder).
fn mpint_bits(value: &[u8]) -> u32 {
    let Some(first) = value.iter().position(|byte| *byte != 0) else {
        return 0;
    };
    let significant = &value[first..];
    (significant.len() as u32 - 1) * 8 + (8 - significant[0].leading_zeros())
}

/// Significant bits for a key type given the parsed blob fields.
fn key_bits(key_type: &str, fields: &[&[u8]]) -> Option<u32> {
    let base = key_type.strip_suffix("-cert-v01@openssh.com").unwrap_or(key_type);
    match base {
        "ssh-rsa" => fields.get(2).map(|field| mpint_bits(field)),
        "ssh-dss" => fields.get(1).map(|field| mpint_bits(field)),
        "ssh-ed25519" | "sk-ssh-ed25519@openssh.com" => Some(256),
        "sk-ecdsa-sha2-nistp256@openssh.com" => Some(256),
        "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => {
            fields.get(1).and_then(|field| match *field {
                b"nistp256" => Some(256),
                b"nistp384" => Some(384),
                b"nistp521" => Some(521),
                _ => None,
            })
        }
        _ => None,
    }
}

/// Group valid entries by machine hint, named machines first.
fn summarize_machines(files: &[SshAuthorizedKeysFile]) -> Vec<SshMachineSummary> {
    let mut counts: HashMap<Option<String>, u32> = HashMap::new();
    for file in files {
        for entry in &file.entries {
            if entry.valid {
                *counts.entry(entry.machine.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut machines: Vec<SshMachineSummary> = counts
        .into_iter()
        .map(|(name, key_count)| SshMachineSummary { name, key_count })
        .collect();
    machines.sort_by(|a, b| match (&a.name, &b.name) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    machines
}

fn home_dir() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn current_user() -> Option<String> {
    let variable = if cfg!(windows) { "USERNAME" } else { "USER" };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.into_string().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_string(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(value);
    }

    fn mpint(value: &[u8]) -> Vec<u8> {
        let first = value.iter().position(|byte| *byte != 0).unwrap_or(0);
        let mut bytes = value[first..].to_vec();
        if bytes.first().is_some_and(|byte| byte & 0x80 != 0) {
            bytes.insert(0, 0);
        }
        bytes
    }

    fn ed25519_key() -> String {
        let mut blob = Vec::new();
        ssh_string(&mut blob, b"ssh-ed25519");
        ssh_string(&mut blob, &[0x42; 32]);
        STANDARD.encode(&blob)
    }

    fn rsa_key(modulus: &[u8]) -> String {
        let mut blob = Vec::new();
        ssh_string(&mut blob, b"ssh-rsa");
        ssh_string(&mut blob, &mpint(&[0x01, 0x00, 0x01]));
        ssh_string(&mut blob, &mpint(modulus));
        STANDARD.encode(&blob)
    }

    #[test]
    fn parses_plain_keys_and_infers_machines_from_comments() {
        let content = format!(
            "# backup of old keys\n\nssh-ed25519 {} owner@ThinkPad\n",
            ed25519_key()
        );
        let entries = parse_authorized_keys(&content);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.line, 3);
        assert!(entry.valid, "unexpected error: {:?}", entry.error);
        assert_eq!(entry.key_type.as_deref(), Some("ssh-ed25519"));
        assert_eq!(entry.key_bits, Some(256));
        assert_eq!(entry.machine.as_deref(), Some("thinkpad"));
        assert_eq!(entry.comment.as_deref(), Some("owner@ThinkPad"));
        assert!(
            entry
                .fingerprint
                .as_deref()
                .is_some_and(|value| value.starts_with("SHA256:"))
        );
    }

    #[test]
    fn parses_options_with_quoted_commas_and_restrictions() {
        let content = format!(
            "from=\"10.0.0.* , 192.168.1.20\",command=\"/usr/local/bin/backup --daily, nightly\",no-pty ssh-ed25519 {} ci@buildbox\n",
            ed25519_key()
        );
        let entries = parse_authorized_keys(&content);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!(entry.valid, "unexpected error: {:?}", entry.error);
        assert_eq!(entry.options.len(), 3);
        assert_eq!(
            entry.from_patterns,
            Some(vec!["10.0.0.*".to_owned(), "192.168.1.20".to_owned()])
        );
        assert_eq!(
            entry.forced_command.as_deref(),
            Some("/usr/local/bin/backup --daily, nightly")
        );
        assert_eq!(entry.machine.as_deref(), Some("buildbox"));
    }

    #[test]
    fn separates_options_from_key_type_without_whitespace() {
        let content = format!("restrict,ssh-rsa {} legacy@server\n", rsa_key(&[0xC0; 256]));
        let entries = parse_authorized_keys(&content);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!(entry.valid, "unexpected error: {:?}", entry.error);
        assert_eq!(entry.options, vec!["restrict".to_owned()]);
        assert_eq!(entry.key_bits, Some(2048));
        assert_eq!(entry.machine.as_deref(), Some("server"));
    }

    #[test]
    fn reports_invalid_lines_and_keeps_line_numbers() {
        let content = format!(
            "ssh-ed25519 {} owner@laptop\nthis is garbage\nssh-ed25519 AAAA owner@laptop\n",
            ed25519_key()
        );
        let entries = parse_authorized_keys(&content);
        assert_eq!(entries.len(), 3);

        assert!(entries[0].valid);
        assert_eq!(entries[0].line, 1);

        assert!(!entries[1].valid);
        assert_eq!(entries[1].line, 2);
        assert!(entries[1].error.is_some());

        // Decodes as base64 but the blob is not a well-formed ed25519 key.
        assert!(!entries[2].valid);
        assert_eq!(entries[2].line, 3);
        assert!(entries[2].error.is_some());
    }

    #[test]
    fn machine_hint_requires_user_at_host_shape() {
        assert_eq!(
            machine_hint(Some("owner@laptop work machine")),
            Some("laptop".to_owned())
        );
        assert_eq!(machine_hint(Some("owner@")), None);
        assert_eq!(machine_hint(Some("recovery key 2024")), None);
        assert_eq!(machine_hint(Some("")), None);
        assert_eq!(machine_hint(None), None);
    }

    #[test]
    fn scans_a_home_directory_and_summarizes_machines() {
        let home = tempfile::tempdir().expect("tempdir");
        let ssh_dir = home.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).expect("ssh dir");
        let modulus = vec![0xA5; 384];
        std::fs::write(
            ssh_dir.join("authorized_keys"),
            format!(
                "ssh-ed25519 {} owner@thinkpad\nssh-rsa {} owner@thinkpad\nbackup-key\n",
                ed25519_key(),
                rsa_key(&modulus),
            ),
        )
        .expect("write keys");

        let report = scan_with_home(Some(home.path()), Some("owner".to_owned()), "linux");
        assert_eq!(report.platform, "linux");
        assert_eq!(report.user.as_deref(), Some("owner"));
        assert_eq!(report.home_dir, Some(home.path().to_string_lossy().into_owned()));

        let user_file = report
            .files
            .iter()
            .find(|file| file.role == SshKeyFileRole::User)
            .expect("user file");
        assert!(user_file.exists);
        assert_eq!(user_file.entries.len(), 3);
        assert_eq!(user_file.entries.iter().filter(|entry| entry.valid).count(), 2);
        assert_eq!(user_file.entries[1].key_bits, Some(3072));

        // Machine summaries are exact when only the temp home file is considered.
        assert_eq!(
            summarize_machines(&report.files[0..1]),
            vec![SshMachineSummary {
                name: Some("thinkpad".to_owned()),
                key_count: 2
            }]
        );
    }

    #[test]
    fn missing_home_reports_a_placeholder_file() {
        let report = scan_with_home(None, None, "linux");
        let user_file = report
            .files
            .iter()
            .find(|file| file.role == SshKeyFileRole::User)
            .expect("user file");
        assert_eq!(user_file.path, "~/.ssh/authorized_keys");
        assert!(!user_file.exists);
        assert!(user_file.error.is_some());
    }
}
