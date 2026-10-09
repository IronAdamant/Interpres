//! Hand-written settings (no serde). Stored as simple `key=value` lines.

use crate::theme::ThemeMode;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Runtime configuration for Interpres.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// When true, write session transcripts to disk (default on: the point is keeping notes).
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
    /// Caption source: `os` (Live Captions, default), `engine` (external speech engine at
    /// `helper_path` + `helper_args`, see docs/ENGINES.md), or `demo`.
    pub source: String,
    /// Arguments for the external engine (quotes group words: `-u "C:\my engine.py"`).
    pub helper_args: String,
    /// Write debug logs into the transcript folder (`interpres-debug.log` / session `.debug.log`).
    pub debug: bool,
    /// UI appearance: system (follow OS), light, or dark. Does not affect capture.
    pub theme: ThemeMode,
    /// Ask "are you done?" after this many minutes with no new captions (0 = never).
    /// Recording never stops on its own (unless `auto_record` is on).
    pub idle_prompt_minutes: u64,
    /// Start recording when sound plays through the speakers, and stop + save after
    /// `auto_stop_quiet_minutes` of silence (Windows). Off by default.
    pub auto_record: bool,
    /// With `auto_record` on: stop and save after this many minutes of no sound (0 = never).
    pub auto_stop_quiet_minutes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            remember: true,
            transcript_folder: default_transcript_folder(),
            write_jsonl: false,
            // LC process detection can blip under load; keep debounce ≥ lifecycle floor.
            off_delay_ms: 3500,
            // Fast poll: short LC lines appear and leave quickly (CPU trade is intentional).
            poll_ms: 150,
            helper_path: None,
            source: "os".to_string(),
            helper_args: String::new(),
            debug: false,
            theme: ThemeMode::System,
            idle_prompt_minutes: 3,
            auto_record: false,
            auto_stop_quiet_minutes: 5,
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
    /// True when the user chose an external speech engine and configured its path.
    pub fn uses_external_engine(&self) -> bool {
        self.source == "engine" && self.helper_path.is_some()
    }

    /// Short display name for the external engine (file stem of the script or program).
    pub fn engine_name(&self) -> String {
        let args = split_args(&self.helper_args);
        // `python -u engine.py` → "engine"; a direct exe → its own stem.
        let script = args.iter().find(|a| {
            let l = a.to_ascii_lowercase();
            l.ends_with(".py") || l.ends_with(".js") || l.ends_with(".ps1") || l.ends_with(".sh")
        });
        let path = script
            .map(PathBuf::from)
            .or_else(|| self.helper_path.clone())
            .unwrap_or_default();
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "engine".into())
    }

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
                "helper_args" => cfg.helper_args = v.to_string(),
                "source" => {
                    if !v.is_empty() {
                        cfg.source = v.to_string();
                    }
                }
                "debug" => cfg.debug = parse_bool(v),
                "theme" => cfg.theme = ThemeMode::parse(v),
                "idle_prompt_minutes" => {
                    if let Ok(n) = v.parse() {
                        cfg.idle_prompt_minutes = n;
                    }
                }
                "auto_record" => cfg.auto_record = parse_bool(v),
                "auto_stop_quiet_minutes" => {
                    if let Ok(n) = v.parse() {
                        cfg.auto_stop_quiet_minutes = n;
                    }
                }
                _ => {}
            }
        }
        cfg.migrate_old_defaults();
        cfg
    }

    /// Early builds saved their defaults into the file, and every save rewrites every
    /// key, so those values stuck. Replace exactly those old defaults with today's.
    fn migrate_old_defaults(&mut self) {
        // poll_ms default was 400 until 2026-08-07 (now 150: short lines leave quickly).
        if self.poll_ms == 400 {
            self.poll_ms = Config::default().poll_ms;
        }
        // off_delay_ms default was 2500 until 2026-08-06 (now 3500: LC detection blips).
        if self.off_delay_ms == 2500 {
            self.off_delay_ms = Config::default().off_delay_ms;
        }
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
        writeln!(f, "helper_args={}", self.helper_args)?;
        writeln!(f, "source={}", self.source)?;
        writeln!(f, "debug={}", if self.debug { "true" } else { "false" })?;
        writeln!(f, "theme={}", self.theme.as_str())?;
        writeln!(f, "idle_prompt_minutes={}", self.idle_prompt_minutes)?;
        writeln!(
            f,
            "auto_record={}",
            if self.auto_record { "true" } else { "false" }
        )?;
        writeln!(f, "auto_stop_quiet_minutes={}", self.auto_stop_quiet_minutes)?;
        Ok(())
    }
}

/// Split a command-line style argument string. Double quotes group words; no escapes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
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
    fn old_saved_defaults_are_migrated() {
        let path = temp_conf();
        fs::write(&path, "poll_ms=400\noff_delay_ms=2500\n").unwrap();
        let cfg = Config::load_from(&path);
        assert_eq!(cfg.poll_ms, Config::default().poll_ms);
        assert_eq!(cfg.off_delay_ms, Config::default().off_delay_ms);
        // A value the user picked on purpose is kept.
        fs::write(&path, "poll_ms=250\noff_delay_ms=5000\n").unwrap();
        let cfg = Config::load_from(&path);
        assert_eq!((cfg.poll_ms, cfg.off_delay_ms), (250, 5000));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn split_args_handles_quoted_paths() {
        assert_eq!(
            split_args(r#"-u "C:\My Engines\phonon.py" --model  C:\models\p2"#),
            vec!["-u", r"C:\My Engines\phonon.py", "--model", r"C:\models\p2"]
        );
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args(r#"a "" b"#), vec!["a", "", "b"]);
    }

    #[test]
    fn external_engine_settings_roundtrip() {
        let path = temp_conf();
        let mut cfg = Config::default();
        cfg.source = "engine".into();
        cfg.helper_path = Some(PathBuf::from(r"C:\Python313\python.exe"));
        cfg.helper_args = r#"-u "C:\engines\phonon_engine.py""#.into();
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        assert!(loaded.uses_external_engine());
        assert_eq!(loaded.helper_args, cfg.helper_args);
        assert_eq!(loaded.engine_name(), "phonon_engine");
        let _ = fs::remove_file(path);
        assert!(!Config::default().uses_external_engine());
    }

    #[test]
    fn saving_defaults_on_but_respects_explicit_off() {
        assert!(Config::default().remember, "new installs save transcripts");
        let path = temp_conf();
        fs::write(&path, "remember=false
").unwrap();
        assert!(!Config::load_from(&path).remember, "existing OFF choice kept");
        fs::write(&path, "theme=dark
").unwrap();
        assert!(Config::load_from(&path).remember, "missing key → default on");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn roundtrip_sticky_folder_and_remember() {
        let path = temp_conf();
        let mut cfg = Config::default();
        cfg.remember = false;
        cfg.transcript_folder = PathBuf::from("/Users/example/My Captions");
        cfg.write_jsonl = true;
        cfg.off_delay_ms = 3000;
        cfg.source = "os".into();
        cfg.theme = ThemeMode::Light;
        cfg.auto_record = true;
        cfg.auto_stop_quiet_minutes = 7;
        cfg.save_to(&path).expect("save");
        let loaded = Config::load_from(&path);
        assert_eq!(loaded.remember, false);
        assert_eq!(
            loaded.transcript_folder,
            PathBuf::from("/Users/example/My Captions")
        );
        assert_eq!(loaded.write_jsonl, true);
        assert_eq!(loaded.off_delay_ms, 3000);
        assert_eq!(loaded.theme, ThemeMode::Light);
        assert!(loaded.auto_record);
        assert_eq!(loaded.auto_stop_quiet_minutes, 7);
        assert!(!Config::default().auto_record, "auto-record is opt-in");
        let _ = fs::remove_file(path);
    }
}
