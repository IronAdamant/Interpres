//! Hand-written settings (no serde). Stored as simple `key=value` lines.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Runtime configuration for Interpres.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// When true, write session transcripts to disk.
    pub remember: bool,
    /// Sticky user-chosen folder for transcripts. Empty means default Documents path.
    pub transcript_folder: PathBuf,
    /// Also write companion `.jsonl` next to the human `.txt` file.
    pub write_jsonl: bool,
    /// Lifecycle off-delay in milliseconds before treating Live Captions as stopped.
    pub off_delay_ms: u64,
    /// How often to poll Live Captions process presence (ms).
    pub poll_ms: u64,
    /// Optional override path to a caption helper binary/script.
    pub helper_path: Option<PathBuf>,
    /// Caption source: `os` (default), `demo` (fixture/stdin), or helper path mode.
    pub source: String,
    /// Write debug logs into the transcript folder (`interpres-debug.log` / session `.debug.log`).
    pub debug: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            remember: false,
            transcript_folder: default_transcript_folder(),
            write_jsonl: false,
            // LC process detection can blip under load; keep debounce ≥ lifecycle floor.
            off_delay_ms: 3500,
            // Fast poll: short LC lines appear and leave quickly (CPU trade is intentional).
            poll_ms: 150,
            helper_path: None,
            source: "os".to_string(),
            debug: false,
        }
    }
}

/// Default human-visible folder for transcripts.
pub fn default_transcript_folder() -> PathBuf {
    if let Some(home) = home_dir() {
        // Prefer Documents when present.
        let docs = home.join("Documents").join("Interpres Transcripts");
        if home.join("Documents").is_dir() {
            return docs;
        }
        return home.join("Interpres Transcripts");
    }
    PathBuf::from("Interpres Transcripts")
}

fn home_dir() -> Option<PathBuf> {
    // Do not use external crates; env only.
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Config file path: `~/.config/interpres/settings.conf` or Windows equivalent.
pub fn config_path() -> PathBuf {
    if let Some(home) = home_dir() {
        #[cfg(windows)]
        {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                return PathBuf::from(appdata).join("Interpres").join("settings.conf");
            }
        }
        return home.join(".config").join("interpres").join("settings.conf");
    }
    PathBuf::from("interpres-settings.conf")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Self {
        let mut cfg = Config::default();
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return cfg,
        };
        for line in BufReader::new(file).lines().flatten() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let k = k.trim();
            let v = v.trim();
            match k {
                "remember" => cfg.remember = parse_bool(v),
                "transcript_folder" => {
                    if !v.is_empty() {
                        cfg.transcript_folder = PathBuf::from(v);
                    }
                }
                "write_jsonl" => cfg.write_jsonl = parse_bool(v),
                "off_delay_ms" => {
                    if let Ok(n) = v.parse() {
                        cfg.off_delay_ms = n;
                    }
                }
                "poll_ms" => {
                    if let Ok(n) = v.parse() {
                        cfg.poll_ms = n;
                    }
                }
                "helper_path" => {
                    if v.is_empty() {
                        cfg.helper_path = None;
                    } else {
                        cfg.helper_path = Some(PathBuf::from(v));
                    }
                }
                "source" => {
                    if !v.is_empty() {
                        cfg.source = v.to_string();
                    }
                }
                "debug" => cfg.debug = parse_bool(v),
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        writeln!(f, "# Interpres settings (key=value)")?;
        writeln!(f, "remember={}", if self.remember { "true" } else { "false" })?;
        writeln!(
            f,
            "transcript_folder={}",
            self.transcript_folder.display()
        )?;
        writeln!(
            f,
            "write_jsonl={}",
            if self.write_jsonl { "true" } else { "false" }
        )?;
        writeln!(f, "off_delay_ms={}", self.off_delay_ms)?;
        writeln!(f, "poll_ms={}", self.poll_ms)?;
        writeln!(
            f,
            "helper_path={}",
            self.helper_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        )?;
        writeln!(f, "source={}", self.source)?;
        writeln!(f, "debug={}", if self.debug { "true" } else { "false" })?;
        Ok(())
    }
}

fn parse_bool(v: &str) -> bool {
    matches!(
        v.to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_conf() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("interpres-test-cfg-{n}.conf"))
    }

    #[test]
    fn roundtrip_sticky_folder_and_remember() {
        let path = temp_conf();
        let mut cfg = Config::default();
        cfg.remember = true;
        cfg.transcript_folder = PathBuf::from("/Users/example/My Captions");
        cfg.write_jsonl = true;
        cfg.off_delay_ms = 3000;
        cfg.source = "os".into();
        cfg.save_to(&path).expect("save");
        let loaded = Config::load_from(&path);
        assert_eq!(loaded.remember, true);
        assert_eq!(
            loaded.transcript_folder,
            PathBuf::from("/Users/example/My Captions")
        );
        assert_eq!(loaded.write_jsonl, true);
        assert_eq!(loaded.off_delay_ms, 3000);
        let _ = fs::remove_file(path);
    }
}
