#[cfg_attr(not(unix), allow(dead_code))]
mod sandbox;

#[cfg_attr(not(unix), allow(dead_code))]
mod deployment;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    unix::run().await
}

#[cfg(not(unix))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("soloops-hostd requires a Unix platform and Unix domain sockets")
}
