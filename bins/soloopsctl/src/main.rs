use anyhow::{Context, Result, bail};
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use clap::{Parser, Subcommand};
use soloops_server::AppConfig;
use soloops_storage::Database;

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
    /// Verify configuration and database readiness.
    Doctor,
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
            let password = rpassword::prompt_password("Owner password: ")?;
            let confirmation = rpassword::prompt_password("Confirm password: ")?;
            if password != confirmation {
                bail!("passwords do not match");
            }
            if password.chars().count() < 12 {
                bail!("password must contain at least 12 characters");
            }
            let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>())
                .map_err(|error| anyhow::anyhow!("failed to generate password salt: {error}"))?;
            let password_hash = Argon2::default()
                .hash_password(password.as_bytes(), &salt)
                .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))?
                .to_string();
            let owner = database.create_owner(&username, &password_hash).await?;
            println!("Owner '{}' created.", owner.username);
        }
        Command::Doctor => {
            database.ready().await?;
            println!("SoloOps configuration and database are ready.");
        }
    }
    database.close().await;
    Ok(())
}
