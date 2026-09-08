use anyhow::{Context, Result, bail};
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use clap::{Parser, Subcommand};
use soloops_server::AppConfig;
use soloops_storage::Database;

const PASSWORD_MIN_CHARS: usize = 12;
const MAX_PASSWORD_ATTEMPTS: usize = 3;

#[derive(Debug, Parser)]
#[command(name = "soloopsctl", about = "SoloOps administrative command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply explicit, backward-compatible database migrations.
    Migrate,
    /// Initialize the single Owner account.
    OwnerInit {
        #[arg(long, default_value = "owner")]
        username: String,
    },
    /// Reset the Owner password from the local terminal; revokes all sessions.
    OwnerPasswordReset {
        #[arg(long, default_value = "owner")]
        username: String,
    },
    /// Verify configuration and database readiness.
    Doctor,
}

/// Validates an Owner password candidate against the password policy.
/// Returned text is user-facing feedback shown on the next retry.
fn validate_owner_password(password: &str) -> Result<(), &'static str> {
    let length = password.chars().count();
    if length < PASSWORD_MIN_CHARS {
        return Err("password must contain at least 12 characters");
    }
    Ok(())
}

/// Prompts interactively until both the policy check and the confirmation
/// pass, or until the attempt budget is exhausted. Every failure explains
/// the rule and how many attempts remain, so a typo never ends the flow.
fn prompt_owner_password(label: &str) -> Result<String> {
    for attempt in 1..=MAX_PASSWORD_ATTEMPTS {
        let password =
            rpassword::prompt_password(format!("{label} (at least {PASSWORD_MIN_CHARS} characters): "))?;
        if let Err(reason) = validate_owner_password(&password) {
            eprintln!("Attempt {attempt}/{MAX_PASSWORD_ATTEMPTS}: {reason}. Try again.");
            continue;
        }
        let confirmation = rpassword::prompt_password("Confirm password: ")?;
        if password != confirmation {
            eprintln!("Attempt {attempt}/{MAX_PASSWORD_ATTEMPTS}: passwords do not match. Try again.");
            continue;
        }
        return Ok(password);
    }
    bail!("password entry failed after {MAX_PASSWORD_ATTEMPTS} attempts")
}

fn hash_owner_password(password: &str) -> Result<String> {
    let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>())
        .map_err(|error| anyhow::anyhow!("failed to generate password salt: {error}"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))?
        .to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = AppConfig::load()?;
    let database = Database::connect(&config.database_path).await?;
    match cli.command {
        Command::Migrate => {
            database.migrate().await?;
            println!("Database migrations are up to date.");
        }
        Command::OwnerInit { username } => {
            database
                .verify_schema()
                .await
                .context("run `soloopsctl migrate` before owner initialization")?;
            if database.count_users().await? > 0 {
                bail!("Owner already exists");
            }
            let password = prompt_owner_password("Owner password")?;
            let password_hash = hash_owner_password(&password)?;
            let owner = database.create_owner(&username, &password_hash).await?;
            println!("Owner '{}' created.", owner.username);
        }
        Command::OwnerPasswordReset { username } => {
            database
                .verify_schema()
                .await
                .context("run `soloopsctl migrate` before password reset")?;
            let user = database
                .find_user_by_username(&username)
                .await?
                .with_context(|| format!("Owner '{username}' not found"))?;
            let password = prompt_owner_password("New owner password")?;
            let password_hash = hash_owner_password(&password)?;
            database
                .replace_owner_password(&user.id, &password_hash, "cli", "auth.password_reset")
                .await?;
            println!(
                "Password for '{}' updated; all sessions have been revoked.",
                user.username
            );
        }
        Command::Doctor => {
            database.ready().await?;
            println!("SoloOps configuration and database are ready.");
        }
    }
    database.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_passwords_with_actionable_feedback() {
        assert_eq!(
            validate_owner_password(""),
            Err("password must contain at least 12 characters")
        );
        assert_eq!(
            validate_owner_password("short"),
            Err("password must contain at least 12 characters")
        );
        assert_eq!(
            validate_owner_password("eleven_char"),
            Err("password must contain at least 12 characters")
        );
        assert!(
            validate_owner_password("twelve_chars").is_ok(),
            "exactly twelve characters passes"
        );
        assert!(validate_owner_password("a very long passphrase").is_ok());
    }
}
