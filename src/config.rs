//! Defaults from `~/.ouirc`: `key = value` lines as in `plcrc`. Every key
//! mirrors a flag, and flags win.

use std::env;
use std::fs;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;

use crate::extras::Extra;
use crate::keyboard::Layout;
use crate::words::Mode;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub language: Option<String>,
    pub mode: Option<Mode>,
    pub with: Vec<Extra>,
    pub words: Option<NonZeroUsize>,
    pub time: Option<NonZeroU64>,
    /// `None` also for `layout = auto`, which detects it from the words.
    pub layout: Option<Layout>,
}

/// `~/.ouirc`.
pub fn path() -> Result<PathBuf> {
    let home = env::var_os("HOME").context("HOME not set")?;
    Ok(Path::new(&home).join(".ouirc"))
}

impl Config {
    /// The config at `path`; a missing file is an empty one.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                parse(&text).map_err(|e| anyhow!("{}: {e:#}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Config::default())
            }
            Err(e) => {
                Err(e).with_context(|| format!("reading {}", path.display()))
            }
        }
    }
}

/// Parses config text; `#` comments, blank lines, a leading `export ` and
/// quotes around values are tolerated, unknown keys are not.
pub fn parse(text: &str) -> Result<Config> {
    let mut config = Config::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            bail!("line {}: expected `key = value`", n + 1);
        };
        let value = value.trim().trim_matches(['"', '\'']).trim();
        set(&mut config, key.trim(), value)
            .with_context(|| format!("line {}", n + 1))?;
    }
    if config.words.is_some() && config.time.is_some() {
        bail!("set words or time, not both");
    }
    Ok(config)
}

fn set(config: &mut Config, key: &str, value: &str) -> Result<()> {
    match key {
        "language" => config.language = Some(value.to_string()),
        "mode" => config.mode = Some(choice(value)?),
        "with" => {
            let names =
                value.split(',').map(str::trim).filter(|v| !v.is_empty());
            config.with = names.map(choice).collect::<Result<_>>()?;
        }
        "words" => config.words = Some(number(value)?),
        "time" => config.time = Some(number(value)?),
        "layout" if value == "auto" => config.layout = None,
        "layout" => config.layout = Some(choice(value)?),
        _ => bail!("unknown key '{key}'"),
    }
    Ok(())
}

fn choice<T: ValueEnum>(value: &str) -> Result<T> {
    T::from_str(value, true).map_err(|_| {
        let names: Vec<String> = T::value_variants()
            .iter()
            .filter_map(|v| Some(v.to_possible_value()?.get_name().to_string()))
            .collect();
        anyhow!("'{value}' is not one of {}", names.join(", "))
    })
}

fn number<T: std::str::FromStr>(value: &str) -> Result<T> {
    value.parse().map_err(|_| anyhow!("'{value}' is not a positive number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_parses() {
        let text = "
            # defaults for oui
            language = russian
            mode     = words
            with     = punctuation, n
            words    = 30
            layout   = jcuken
        ";
        let config = parse(text).unwrap();
        assert_eq!(config.language.as_deref(), Some("russian"));
        assert_eq!(config.mode, Some(Mode::Words));
        assert_eq!(config.with, [Extra::Punctuation, Extra::Numbers]);
        assert_eq!(config.words, NonZeroUsize::new(30));
        assert_eq!(config.layout, Some(Layout::Jcuken));
    }

    #[test]
    fn plcrc_style_lines_are_tolerated() {
        let config =
            parse("export time = '60'\nlayout=\"auto\"\nwith =\n").unwrap();
        assert_eq!(config.time, NonZeroU64::new(60));
        assert_eq!(config.layout, None);
        assert!(config.with.is_empty());
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn mistakes_name_their_line() {
        let err = |text: &str| format!("{:#}", parse(text).unwrap_err());
        assert_eq!(err("# ok\nlang = fr"), "line 2: unknown key 'lang'");
        assert_eq!(err("words = 0"), "line 1: '0' is not a positive number");
        assert!(err("mode = prose")
            .starts_with("line 1: 'prose' is not one of words, text"));
        assert_eq!(err("words\n"), "line 1: expected `key = value`");
        assert_eq!(err("words = 5\ntime = 9"), "set words or time, not both");
    }

    #[test]
    fn a_missing_file_is_an_empty_config() {
        let config =
            Config::load(Path::new("/nonexistent/oui/config")).unwrap();
        assert_eq!(config, Config::default());
    }
}
