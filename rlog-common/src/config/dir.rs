//! Load a bunch of fragmented config file into a single structure
//!
//! The idea is to start from a default config (thus `C` must implements `Default`)
//! and extend it though file from a directory.
//!
//! Files in the directory will be watch for modification,
//! and config will be hot reloaded from all files (and not just
//! extended with new or modified files)

use anyhow::{bail, Context};
use arc_swap::ArcSwap;
use glob::{glob_with, MatchOptions};
use serde::{de::DeserializeOwned, Serialize};
use std::{iter::once, path::Path, sync::Arc};
use tokio::{
    sync::watch::{self, Receiver},
    time::sleep,
};

use crate::{
    config::{load_config, CONFIG_REFRESH_INTERVAL},
    utils::format_error,
};

/// Load all the config files from the given directory.
///
/// If the directory does not exists, a warning will be issued but no error will be returned.
///
/// `glob` is a pattern used to match files in the directory. The exact syntax
/// can be found in the [`glob` crate documentation](https://docs.rs/glob/0.3.1/glob/)
///
/// Absolute `glob` is not allowed.
pub fn setup_config_from_dir<C, D>(
    directory: D,
    glob: &str,
    config_store: &'static ArcSwap<C>,
) -> anyhow::Result<Receiver<()>>
where
    C: DeserializeOwned + Serialize + Send + Sync + Default + Extend<C> + Eq,
    D: AsRef<Path>,
{
    if glob.starts_with("/") {
        bail!("Absolute pattern `{glob}` is not allowed")
    }
    let glob = directory
        .as_ref()
        .to_owned()
        .join(glob)
        .to_string_lossy()
        .into_owned();
    tracing::debug!("Config file glob pattern: {glob}");

    let initial_config = read_config(&glob)?;

    config_store.swap(Arc::new(initial_config));

    let (sender, receiver) = watch::channel(());
    tokio::spawn(async move {
        let glob = glob;
        loop {
            sleep(CONFIG_REFRESH_INTERVAL).await;
            match read_config::<C>(&glob) {
                Ok(new_config) => {
                    if &new_config != config_store.load().as_ref() {
                        // new config!!
                        tracing::debug!("Refreshed configuration from {glob}");
                        config_store.store(Arc::new(new_config));
                        if let Err(e) = sender.send(()) {
                            // channel closed,
                            return;
                        }
                    }
                }
                Err(e) => tracing::error!(
                    "Unable to read configuration from {glob}: {}",
                    format_error(e)
                ),
            }
        }
    });

    Ok(receiver)
}

fn read_config<C>(glob: &str) -> Result<C, anyhow::Error>
where
    C: DeserializeOwned + Serialize + Send + Sync + Default + Extend<C> + Eq,
{
    let mut root_config = C::default();
    for path in glob_with(
        &glob,
        MatchOptions {
            require_literal_leading_dot: true,
            ..Default::default()
        },
    )
    .context("Invalid config glob pattern!")?
    {
        if let Ok(path) = path {
            tracing::debug!("Found config file {}", path.to_string_lossy());
            let (config, _) = load_config::<_, C>(&path)?;
            root_config.extend(once(config));
        }
    }
    tracing::debug!("Final config: {}", serde_yaml::to_string(&root_config)?);
    Ok(root_config)
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, fs::File, io::Write, sync::Arc};

    use arc_swap::ArcSwap;
    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    #[derive(Serialize, Deserialize, Default, PartialEq, Eq)]
    struct TestConfig(HashMap<String, String>);

    impl Extend<TestConfig> for TestConfig {
        fn extend<T: IntoIterator<Item = TestConfig>>(&mut self, iter: T) {
            for c in iter {
                self.0.extend(c.0);
            }
        }
    }

    #[tokio::test]
    async fn test() {
        let dir = tempdir().unwrap();

        // lol, you should probably not use that in real code ;)
        let config: &'static ArcSwap<TestConfig> =
            Box::leak(Box::new(ArcSwap::new(Arc::new(Default::default()))));

        // let's try with an empty dir
        super::setup_config_from_dir(dir.path(), "*.yml", config).expect("Empty dir");

        // let's write some sample config file
        write!(
            File::create(dir.path().join(".hidden.yml")).unwrap(),
            "hidden: this is hidden"
        )
        .unwrap();
        write!(
            File::create(dir.path().join("not-the-right-ext.xml")).unwrap(),
            "xml: foo"
        )
        .unwrap();
        write!(
            File::create(dir.path().join("01.yml")).unwrap(),
            "first: foo\nmultiple: let's"
        )
        .unwrap();
        write!(
            File::create(dir.path().join("02.yml")).unwrap(),
            "second: foobar\nthird: bar\nmultiple: let's go!"
        )
        .unwrap();

        super::setup_config_from_dir(dir.path(), "*.yml", config).expect("Cannot load from sample");

        assert!(!config.load().0.contains_key("hidden"));
        assert!(!config.load().0.contains_key("xml"));

        assert_eq!(
            config.load().0.get("first").map(String::as_str),
            Some("foo")
        );
        assert_eq!(
            config.load().0.get("second").map(String::as_str),
            Some("foobar")
        );
        assert_eq!(
            config.load().0.get("third").map(String::as_str),
            Some("bar")
        );
        assert_eq!(
            config.load().0.get("multiple").map(String::as_str),
            Some("let's go!")
        );

        // let's reload the config
        write!(
            File::create(dir.path().join("01.yml")).unwrap(),
            "first-plop: foobar"
        )
        .unwrap();
        super::setup_config_from_dir(dir.path(), "*.yml", config).expect("Cannot load from sample");
        assert!(!config.load().0.contains_key("first"));
        assert_eq!(
            config.load().0.get("first-plop").map(String::as_str),
            Some("foobar")
        );

        assert_eq!(
            config.load().0.get("second").map(String::as_str),
            Some("foobar")
        );
        assert_eq!(
            config.load().0.get("third").map(String::as_str),
            Some("bar")
        );
    }
}
