use std::{
    fs::File,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::Context;
use arc_swap::ArcSwap;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::watch::{self, Receiver};

use crate::utils::format_error;

const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub mod dir;

pub fn setup_config_from_file<C: DeserializeOwned + Serialize + Send + Sync>(
    path: &str,
    config: &'static ArcSwap<C>,
) -> anyhow::Result<Receiver<()>> {
    let mut last_modified = load_and_swap_config(path, config)?;

    let (sender, receiver) = watch::channel(());

    let path = path.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CONFIG_REFRESH_INTERVAL).await;
            if let Some(modified) = std::fs::metadata(&path).and_then(|m| m.modified()).ok() {
                if modified != last_modified {
                    tracing::info!("Config file modified, reloading it!");
                    match load_and_swap_config(&path, &config) {
                        Ok(m) => {
                            last_modified = m;
                            tracing::info!(
                                "New config:\n{}",
                                serde_yaml::to_string(config.load().as_ref()).unwrap()
                            );
                            if let Err(_) = sender.send(()) {
                                // channel closed!
                                return;
                            }
                        }
                        Err(e) => tracing::error!("Unable to reload config: {}", format_error(e)),
                    }
                }
            }
        }
    });

    Ok(receiver)
}

fn load_and_swap_config<P: AsRef<Path>, C: DeserializeOwned>(
    path: P,
    config_store: &ArcSwap<C>,
) -> anyhow::Result<SystemTime> {
    let (config, last_modified) = load_config::<P, C>(path)?;

    config_store.swap(Arc::new(config));

    Ok(last_modified)
}

fn load_config<P: AsRef<Path>, C: DeserializeOwned>(path: P) -> anyhow::Result<(C, SystemTime)> {
    let file = File::open(path.as_ref()).with_context(|| {
        format!(
            "Cannot open config file at: {}",
            path.as_ref().to_string_lossy()
        )
    })?;

    let last_modified = file.metadata()?.modified()?;

    Ok((
        serde_yaml::from_reader(file).with_context(|| {
            format!(
                "Invalid YAML in config file at: {}",
                path.as_ref().to_string_lossy()
            )
        })?,
        last_modified,
    ))
}
