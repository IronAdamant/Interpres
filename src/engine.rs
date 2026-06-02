//! Shared core engine: thin-client server, capture, plugins, utterance processing.
//! Used by headless `main` and GUI without duplicating setup.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

use crate::audio::pcm_f32_from_base64;
use crate::audio::{CaptureConfig, Utterance};
use crate::plugins::discover::register_discovered;
use crate::plugins::{PluginMessage, PluginRegistry};
use crate::server::{SubtitleSpeaker, ThinClientServerHandle, TranscriptionUpdate};
use crate::state::ReplyMode;
use crate::util::config::AppConfig;
use crate::util::shutdown::{self, ShutdownFlag};
use crate::util::{paths, Result};

const DEV_TRANSCRIBER: &str = "dev-transcriber";
const DEV_TTS: &str = "dev-tts";

/// Plugin health snapshot for the control UI.
#[derive(Clone, Debug)]
pub struct PluginStatusInfo {
    pub name: String,
    pub healthy: bool,
    pub restarts: u32,
}

/// Capture status snapshot for the control UI (reliable device change feedback).
#[derive(Clone, Debug, Default)]
pub struct CaptureStatus {
    /// The device name currently (attempted to be) used for capture, with IN: prefix if from list.
    pub active_device: String,
    /// True if a working capture stream is active for this device.
    pub healthy: bool,
    /// Last error string if unhealthy (for UI suggestion: check device, permissions, try another).
    pub last_error: Option<String>,
}

/// Live core resources shared with GUI or headless main.
pub struct CoreHandle {
    pub server_handle: Option<Arc<ThinClientServerHandle>>,
    pub broadcast_tx: Option<mpsc::Sender<TranscriptionUpdate>>,
    pub paused: Arc<AtomicBool>,
    pub mic: Arc<AtomicU32>,
    pub mode: Arc<Mutex<ReplyMode>>,
    pub user_name: Arc<Mutex<String>>,
    pub input_device: Arc<Mutex<String>>,
    pub output_device: Arc<Mutex<String>>,
    /// Send user text from the composer ("Speak on my behalf").
    pub composer_tx: mpsc::Sender<String>,
    /// Request switching capture input (may restart the stream).
    pub input_device_tx: mpsc::Sender<String>,
    pub plugin_status: Arc<Mutex<Vec<PluginStatusInfo>>>,
    pub capture_status: Arc<Mutex<CaptureStatus>>,
    /// When false, utterances are not sent to the transcriber (session not started).
    pub session_active: Arc<AtomicBool>,
    pub shutdown: ShutdownFlag,
}

/// Block until [`shutdown::request`] is called (headless main loop).
pub fn run_until_shutdown(handle: &CoreHandle) {
    if !shutdown::is_requested(&handle.shutdown) {
        shutdown::spawn_stdin_quit_listener(handle.shutdown.clone());
        shutdown::print_headless_help();
    }
    while !shutdown::is_requested(&handle.shutdown) {
        thread::sleep(Duration::from_millis(200));
    }
    println!("[engine] Shutting down — waiting for plugins…");
    thread::sleep(Duration::from_millis(600));
}

/// Start server, plugins, capture, and background processing.
pub fn start_core(server_port: u16, config: AppConfig) -> Result<CoreHandle> {
    let _ = paths::ensure_data_dirs();
    let (server_handle, broadcast_tx) = match crate::server::start(server_port) {
        Ok((h, tx)) => (Some(Arc::new(h)), Some(tx)),
        Err(e) => {
            eprintln!(
                "[engine] thin-client server failed: {} (continuing without broadcast)",
                e
            );
            (None, None)
        }
    };

    let paused = Arc::new(AtomicBool::new(false));
    let mic = Arc::new(AtomicU32::new(0));
    let mode = Arc::new(Mutex::new(config.reply_mode));
    let user_name = Arc::new(Mutex::new(config.user_name.clone()));
    let input_device = Arc::new(Mutex::new(config.input_device.clone()));
    let output_device = Arc::new(Mutex::new(config.output_device.clone()));
    let plugin_status = Arc::new(Mutex::new(Vec::new()));
    let capture_status = Arc::new(Mutex::new(CaptureStatus {
        active_device: config.input_device.clone(),
        healthy: false,
        last_error: None,
    }));
    let session_active = Arc::new(AtomicBool::new(false));
    let shutdown = shutdown::new_flag();

    let (composer_tx, composer_rx) = mpsc::channel();
    let (input_device_tx, input_device_rx) = mpsc::channel();

    let (utt_tx, utt_rx) = mpsc::channel::<Utterance>();
    let stream_error = Arc::new(AtomicBool::new(false));

    let initial_input = config.input_device.clone();

    let shutdown_cap = shutdown.clone();
    spawn_capture_supervisor(
        utt_tx,
        input_device_rx,
        stream_error,
        mic.clone(),
        paused.clone(),
        initial_input,
        shutdown_cap,
        capture_status.clone(),
    );

    let bcast = broadcast_tx.clone();
    let paused_worker = paused.clone();
    let mode_worker = mode.clone();
    let user_worker = user_name.clone();
    let out_dev = output_device.clone();
    let status_worker = plugin_status.clone();
    let session_worker = session_active.clone();
    let shutdown_worker = shutdown.clone();

    thread::spawn(move || {
        let mut reg = PluginRegistry::new();
        let root = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
        let _ = register_discovered(&mut reg, &root);

        let sample_rate = 16000u32;
        let user = user_worker.lock().unwrap().clone();
        let _ = reg.send_to(
            DEV_TRANSCRIBER,
            &PluginMessage::Init {
                sample_rate,
                channels: 1,
                user_name: user.clone(),
            },
        );
        let _ = reg.send_to(
            DEV_TTS,
            &PluginMessage::Init {
                sample_rate,
                channels: 1,
                user_name: user,
            },
        );

        run_processing_loop(
            utt_rx,
            composer_rx,
            reg,
            paused_worker,
            mode_worker,
            user_worker,
            out_dev,
            bcast,
            status_worker,
            session_worker,
            shutdown_worker,
        );
    });

    Ok(CoreHandle {
        server_handle,
        broadcast_tx,
        paused,
        mic,
        mode,
        user_name,
        input_device,
        output_device,
        composer_tx,
        input_device_tx,
        plugin_status,
        capture_status,
        session_active,
        shutdown,
    })
}

fn spawn_capture_supervisor(
    utt_tx: mpsc::Sender<Utterance>,
    input_device_rx: mpsc::Receiver<String>,
    stream_error: Arc<AtomicBool>,
    mic: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    initial_input: String,
    shutdown: ShutdownFlag,
    capture_status: Arc<Mutex<CaptureStatus>>,
) {
    thread::spawn(move || {
        let mut current_device = initial_input;
        let mut active_stream: Option<cpal::Stream> = None;

        let mut open_capture = |device: &str| -> Option<cpal::Stream> {
            let mut cfg = CaptureConfig::default();
            cfg.mic_level_atomic = Some(mic.clone());
            cfg.paused_atomic = Some(paused.clone());
            cfg.stream_error_atomic = Some(stream_error.clone());
            cfg.input_device = Some(device.to_string()).filter(|s| !s.is_empty());
            match crate::audio::start_capture(cfg, utt_tx.clone()) {
                Ok(s) => {
                    stream_error.store(false, Ordering::Relaxed);
                    {
                        let mut st = capture_status.lock().unwrap();
                        st.active_device = device.to_string();
                        st.healthy = true;
                        st.last_error = None;
                    }
                    eprintln!("[engine] capture (re)started on device: {}", device);
                    crate::util::logging::log("info", &format!("capture (re)started on device: {}", device));
                    Some(s)
                }
                Err(e) => {
                    let err_str = format!("{}", e);
                    {
                        let mut st = capture_status.lock().unwrap();
                        st.active_device = device.to_string();
                        st.healthy = false;
                        st.last_error = Some(err_str.clone());
                    }
                    eprintln!("[engine] capture failed for '{}': {}", device, err_str);
                    crate::util::logging::log("error", &format!("capture failed for '{}': {}", device, err_str));
                    None
                }
            }
        };

        // Initial open
        {
            let mut st = capture_status.lock().unwrap();
            st.active_device = current_device.clone();
            st.healthy = false;
            st.last_error = None;
        }
        active_stream = open_capture(&current_device);

        loop {
            if shutdown::is_requested(&shutdown) {
                break;
            }
            while let Ok(dev) = input_device_rx.try_recv() {
                let prev_device = current_device.clone();
                current_device = dev;
                // Only replace stream *after* successful open for user-initiated changes.
                // This keeps the old (good) capture alive if the new choice is bad (e.g. wrong VB-CABLE, permissions, disconnected).
                if let Some(new_stream) = open_capture(&current_device) {
                    active_stream = Some(new_stream);
                    eprintln!("[engine] user-requested device change: {} -> {} (old stream replaced only after success)", prev_device, current_device);
                    crate::util::logging::log("info", &format!("user device change: {} -> {} (success)", prev_device, current_device));
                } else {
                    // Keep old stream; status already reflects error for the attempted device.
                    eprintln!("[engine] device change to '{}' failed — keeping previous capture stream (no disruption to good audio)", current_device);
                    crate::util::logging::log("warn", &format!("device change to '{}' failed — kept previous stream", current_device));
                }
            }
            if stream_error.swap(false, Ordering::Relaxed) {
                eprintln!("[engine] capture stream error — attempting restart on current device");
                crate::util::logging::log("warn", "capture stream error — attempting auto-restart");
                // For auto-error recovery, the stream is already dead/broken, so we drop and retry (may end up None if persistent fail).
                active_stream = None;
                active_stream = open_capture(&current_device);
            }
            let _ = &active_stream;
            thread::sleep(Duration::from_millis(400));
        }
    });
}

fn run_processing_loop(
    utt_rx: mpsc::Receiver<Utterance>,
    composer_rx: mpsc::Receiver<String>,
    mut reg: PluginRegistry,
    paused: Arc<AtomicBool>,
    mode: Arc<Mutex<ReplyMode>>,
    user_name: Arc<Mutex<String>>,
    output_device: Arc<Mutex<String>>,
    broadcast_tx: Option<mpsc::Sender<TranscriptionUpdate>>,
    plugin_status: Arc<Mutex<Vec<PluginStatusInfo>>>,
    session_active: Arc<AtomicBool>,
    shutdown: ShutdownFlag,
) {
    let mut last_partial_id = String::new();
    let mut pending_ai_speaks: HashMap<String, String> = HashMap::new();

    loop {
        if shutdown::is_requested(&shutdown) {
            break;
        }
        while let Ok(text) = composer_rx.try_recv() {
            handle_composer_speak(
                &text,
                &mut reg,
                &mode,
                &user_name,
                &output_device,
                broadcast_tx.as_ref(),
                &mut pending_ai_speaks,
            );
        }

        while let Ok(utt) = utt_rx.try_recv() {
            if paused.load(Ordering::Relaxed) || !session_active.load(Ordering::Relaxed) {
                continue;
            }
            last_partial_id = utt.id.clone();
            process_utterance(&utt, &mut reg, &mode, broadcast_tx.as_ref());
        }

        for (name, m) in reg.poll_all() {
            handle_async_plugin_message(
                &name,
                m,
                &mut reg,
                &output_device,
                broadcast_tx.as_ref(),
                &last_partial_id,
                &mut pending_ai_speaks,
            );
        }

        update_plugin_status(&mut reg, &plugin_status);

        match utt_rx.recv_timeout(Duration::from_millis(120)) {
            Ok(utt) => {
                if !paused.load(Ordering::Relaxed) && session_active.load(Ordering::Relaxed) {
                    last_partial_id = utt.id.clone();
                    process_utterance(&utt, &mut reg, &mode, broadcast_tx.as_ref());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    println!("[engine] Processing loop ended — shutting down plugins");
    let _ = reg.shutdown_all();
}

fn process_utterance(
    utt: &Utterance,
    reg: &mut PluginRegistry,
    mode: &Arc<Mutex<ReplyMode>>,
    broadcast_tx: Option<&mpsc::Sender<TranscriptionUpdate>>,
) {
    let _ = reg.send_to(
        DEV_TRANSCRIBER,
        &PluginMessage::Audio {
            id: utt.id.clone(),
            pcm_base64: utt.to_base64(),
            byte_len: utt.pcm.len() * 4,
        },
    );
    let _ = reg.send_to(
        DEV_TRANSCRIBER,
        &PluginMessage::EndUtterance { id: utt.id.clone() },
    );

    for (_name, m) in reg.poll_all() {
        if let Some(tx) = broadcast_tx {
            emit_transcription_message(tx, &utt.id, m, mode);
        }
    }
}

fn handle_async_plugin_message(
    name: &str,
    m: PluginMessage,
    _reg: &mut PluginRegistry,
    output_device: &Arc<Mutex<String>>,
    broadcast_tx: Option<&mpsc::Sender<TranscriptionUpdate>>,
    fallback_id: &str,
    pending_ai: &mut HashMap<String, String>,
) {
    match m {
        PluginMessage::AudioOut {
            id,
            pcm_base64,
            sample_rate,
            byte_len,
        } if name == DEV_TTS => {
            if let Ok(pcm) = pcm_f32_from_base64(&pcm_base64) {
                if pcm.len() * 4 == byte_len || byte_len == 0 {
                    let out = output_device.lock().unwrap().clone();
                    if out.is_empty() {
                        let _ = crate::audio::spawn_playback(pcm.clone(), sample_rate);
                    } else {
                        let _ = crate::audio::spawn_playback_on_device(pcm.clone(), sample_rate, &out);
                    }
                }
            }
            // On AUDIO_OUT success (TTS produced audio): send the *real* pending Ai-labeled update
            // (with original composer text + [AI] + announcement prefix). Guard: no dummy sent.
            // Playback is now non-blocking via spawn (no stall on processing thread).
            if let Some(tx) = broadcast_tx {
                if let Some(text) = pending_ai.remove(&id) {
                    let _ = tx.send(TranscriptionUpdate {
                        id,
                        text,
                        is_final: true,
                        timestamp_ms: current_timestamp_ms(),
                        speaker: SubtitleSpeaker::Ai,
                    });
                }
                // If no pending entry, do not inject anything (e.g. TTS error path sent ERROR not AUDIO_OUT).
            }
        }
        PluginMessage::Partial { id, text } => {
            if let Some(tx) = broadcast_tx {
                let _ = tx.send(TranscriptionUpdate {
                    id: if id.is_empty() { fallback_id.to_string() } else { id },
                    text,
                    is_final: false,
                    timestamp_ms: current_timestamp_ms(),
                    speaker: SubtitleSpeaker::Them,
                });
            }
        }
        PluginMessage::Final { id, text } => {
            if let Some(tx) = broadcast_tx {
                let _ = tx.send(TranscriptionUpdate {
                    id: if id.is_empty() { fallback_id.to_string() } else { id },
                    text,
                    is_final: true,
                    timestamp_ms: current_timestamp_ms(),
                    speaker: SubtitleSpeaker::Them,
                });
            }
        }
        _ => {}
    }
}

fn emit_transcription_message(
    tx: &mpsc::Sender<TranscriptionUpdate>,
    utt_id: &str,
    m: PluginMessage,
    mode: &Arc<Mutex<ReplyMode>>,
) {
    let current_mode = *mode.lock().unwrap();
    match m {
        PluginMessage::Partial { text, .. } => {
            let display = apply_mode_label(&text, current_mode, false);
            let _ = tx.send(TranscriptionUpdate {
                id: utt_id.to_string(),
                text: display,
                is_final: false,
                timestamp_ms: current_timestamp_ms(),
                speaker: SubtitleSpeaker::Them,
            });
        }
        PluginMessage::Final { text, .. } => {
            let display = apply_mode_label(&text, current_mode, false);
            let _ = tx.send(TranscriptionUpdate {
                id: utt_id.to_string(),
                text: display,
                is_final: true,
                timestamp_ms: current_timestamp_ms(),
                speaker: SubtitleSpeaker::Them,
            });
        }
        _ => {}
    }
}

fn apply_mode_label(text: &str, mode: ReplyMode, is_ai: bool) -> String {
    if is_ai {
        return text.to_string();
    }
    match mode {
        ReplyMode::VoiceProtection => format!("[VP] {}", text),
        _ => text.to_string(),
    }
}

fn handle_composer_speak(
    text: &str,
    reg: &mut PluginRegistry,
    mode: &Arc<Mutex<ReplyMode>>,
    user_name: &Arc<Mutex<String>>,
    output_device: &Arc<Mutex<String>>,
    broadcast_tx: Option<&mpsc::Sender<TranscriptionUpdate>>,
    pending_ai: &mut HashMap<String, String>,
) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    let current_mode = *mode.lock().unwrap();
    let user = user_name.lock().unwrap().clone();
    let speak_id = format!("spk-{:x}", current_timestamp_ms());

    let spoken = match current_mode {
        ReplyMode::AiOnBehalf => {
            let prefix = ReplyMode::ai_announcement_prefix(&user);
            format!("{}{}", prefix, trimmed)
        }
        ReplyMode::VoiceProtection => format!("[VP] {}", trimmed),
        ReplyMode::Trusted => trimmed.to_string(),
    };

    if current_mode == ReplyMode::AiOnBehalf {
        // Defer real AI-labeled update until AUDIO_OUT success path (on TTS producing).
        // This aligns with spec: label injected on success, not at Speak time.
        let display = format!("[AI] {}", spoken);
        pending_ai.insert(speak_id.clone(), display.clone());
    } else if let Some(tx) = broadcast_tx {
        // Non-AI composer: immediate as before (speaker Them, text may have [VP])
        let _ = tx.send(TranscriptionUpdate {
            id: speak_id.clone(),
            text: spoken.clone(),
            is_final: true,
            timestamp_ms: current_timestamp_ms(),
            speaker: SubtitleSpeaker::Them,
        });
    }

    if current_mode == ReplyMode::AiOnBehalf {
        let _ = reg.send_to(
            DEV_TTS,
            &PluginMessage::Speak {
                id: speak_id,
                text: spoken,
            },
        );
        let _ = output_device; // used when AUDIO_OUT returns
        for (name, m) in reg.poll_all() {
            handle_async_plugin_message(&name, m, reg, output_device, broadcast_tx, "", pending_ai);
        }
    }
}

fn update_plugin_status(reg: &mut PluginRegistry, out: &Arc<Mutex<Vec<PluginStatusInfo>>>) {
    *out.lock().unwrap() = reg
        .statuses()
        .into_iter()
        .map(|(name, healthy, restarts)| PluginStatusInfo {
            name,
            healthy,
            restarts,
        })
        .collect();
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
