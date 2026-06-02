//! Tiny pure-std HTTP + SSE server for Interpres thin clients (phones/iPads).
//!
//! - Uses only `std::net::TcpListener` + threads (zero external crates).
//! - Serves a single self-contained, beautiful high-contrast subtitles.html.
//! - SSE endpoint `/events` pushes live PARTIAL + FINAL transcriptions in real time.
//! - On startup prints all discoverable local IPs + port for easy phone connection.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver as MpscReceiver, Sender as MpscSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{InterpresError, Result};

/// Who produced this subtitle line (for styling and AI disclosure).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitleSpeaker {
    #[default]
    Them,
    Ai,
    System,
}

impl SubtitleSpeaker {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubtitleSpeaker::Them => "them",
            SubtitleSpeaker::Ai => "ai",
            SubtitleSpeaker::System => "system",
        }
    }
}

/// Event sent from the core transcription pipeline to all connected thin clients.
#[derive(Clone, Debug)]
pub struct TranscriptionUpdate {
    pub id: String,
    pub text: String,
    pub is_final: bool,
    pub timestamp_ms: u64,
    pub speaker: SubtitleSpeaker,
}

/// Maximum number of recent updates retained for late subscribers.
const HISTORY_CAP: usize = 20;

/// Handle to the running thin-client HTTP+SSE server.
///
/// Holds the shared client registry so native overlays and other consumers can
/// `subscribe()` without going through HTTP/SSE.
pub struct ThinClientServerHandle {
    port: u16,
    clients: Arc<Mutex<Vec<MpscSender<TranscriptionUpdate>>>>,
    history: Arc<Mutex<Vec<TranscriptionUpdate>>>,
}

/// Start the server on 0.0.0.0:<port> (use 0 for OS-assigned port).
/// Returns the server handle + a cloneable Sender for broadcasting updates.
/// All connected SSE clients and `subscribe()` receivers will receive every update you send here.
pub fn start(port: u16) -> Result<(ThinClientServerHandle, MpscSender<TranscriptionUpdate>)> {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .map_err(|e| InterpresError::Server(format!("Failed to bind port {}: {}", port, e)))?;

    let actual_port = listener.local_addr()
        .map(|a| a.port())
        .unwrap_or(port);

    // Shared registry of live consumer senders (SSE + native subscribe()).
    // When we broadcast we fan-out to these; dead ones are pruned automatically.
    let clients: Arc<Mutex<Vec<MpscSender<TranscriptionUpdate>>>> =
        Arc::new(Mutex::new(Vec::new()));

    // Rolling buffer for late joiners (overlay opened after speech started).
    let history: Arc<Mutex<Vec<TranscriptionUpdate>>> =
        Arc::new(Mutex::new(Vec::with_capacity(HISTORY_CAP)));

    // Channel for incoming updates from the main transcription flow (fan-out dispatcher).
    let (event_tx, event_rx) = mpsc::channel::<TranscriptionUpdate>();

    // Dispatcher thread: records history, then fans out to all live client channels.
    let clients_for_dispatch = clients.clone();
    let history_for_dispatch = history.clone();
    thread::spawn(move || {
        for update in event_rx {
            {
                let mut hist = history_for_dispatch.lock().unwrap();
                hist.push(update.clone());
                if hist.len() > HISTORY_CAP {
                    let excess = hist.len() - HISTORY_CAP;
                    hist.drain(0..excess);
                }
            }
            let mut live = clients_for_dispatch.lock().unwrap();
            live.retain(|tx| tx.send(update.clone()).is_ok());
        }
    });

    // Acceptor thread: one thread that accepts TCP connections and spawns a handler per client.
    let clients_for_accept = clients.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let clients = clients_for_accept.clone();
                    // Per-connection thread (very cheap for our low-frequency subtitle use case).
                    thread::spawn(move || {
                        if let Err(e) = handle_connection(stream, clients) {
                            // Connection ended or bad request — normal for browsers / phones.
                            if cfg!(debug_assertions) {
                                eprintln!("[server] connection handler: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    if cfg!(debug_assertions) {
                        eprintln!("[server] accept error: {}", e);
                    }
                }
            }
        }
    });

    let server = ThinClientServerHandle {
        port: actual_port,
        clients: clients.clone(),
        history: history.clone(),
    };

    // Friendly startup banner with all the IPs the user needs to type on their phone.
    print_startup_banner(actual_port);

    Ok((server, event_tx))
}

impl ThinClientServerHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Subscribe to live transcription updates (same fan-out as SSE `/events`).
    ///
    /// Returns a receiver; drop it to unregister on the next failed send prune.
    /// Call [`Self::recent_history`] after subscribe if you need prior lines.
    pub fn subscribe(&self) -> MpscReceiver<TranscriptionUpdate> {
        let (tx, rx) = mpsc::channel();
        self.clients.lock().unwrap().push(tx);
        rx
    }

    /// Snapshot of the most recent updates (up to [`HISTORY_CAP`]), oldest first.
    pub fn recent_history(&self) -> Vec<TranscriptionUpdate> {
        self.history.lock().unwrap().clone()
    }
}

/// Best-effort discovery of local IPv4 addresses the phone can reach us on.
/// Always includes 127.0.0.1. The UDP "outbound trick" usually gives the real LAN IP.
fn discover_local_ips() -> Vec<String> {
    let mut ips = vec!["127.0.0.1".to_string()];

    // UDP connect trick — extremely reliable cross-platform way to learn the primary LAN IP.
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        let _ = socket.set_read_timeout(Some(std::time::Duration::from_millis(80)));
        // Try two common public DNS IPs (no packets actually sent for UDP connect).
        let targets = ["8.8.8.8:53", "1.1.1.1:53"];
        for t in targets {
            if socket.connect(t).is_ok() {
                if let Ok(addr) = socket.local_addr() {
                    let s = addr.ip().to_string();
                    if !s.starts_with("127.") && !ips.contains(&s) {
                        ips.push(s);
                    }
                    break;
                }
            }
        }
    }

    // Linux-only: parse /proc/net/fib_trie for additional local /32 addresses (best effort, zero cost).
    if let Ok(content) = std::fs::read_to_string("/proc/net/fib_trie") {
        for line in content.lines() {
            let t = line.trim();
            if (t.contains("/32") || t.contains("LOCAL")) && t.contains('.') {
                if let Some(candidate) = t.split_whitespace().next() {
                    if candidate.chars().filter(|c| *c == '.').count() == 3
                        && candidate.len() <= 15
                        && !candidate.starts_with("127.")
                        && !ips.contains(&candidate.to_string())
                    {
                        ips.push(candidate.to_string());
                    }
                }
            }
        }
    }

    ips
}

fn print_startup_banner(port: u16) {
    let ips = discover_local_ips();

    println!();
    println!("╔════════════════════════════════════════════════════════════════════════════╗");
    println!("║  THIN CLIENT SERVER (pure-std HTTP + SSE)                                  ║");
    println!("╠════════════════════════════════════════════════════════════════════════════╣");
    println!("║  Open this address on any phone or iPad on the same Wi-Fi / LAN:           ║");
    println!("╟────────────────────────────────────────────────────────────────────────────╢");

    for ip in &ips {
        let line = format!("║    http://{}:{}/", ip, port);
        // Pad to look nice in the box
        println!("{:<78}║", line);
        // Also capture the actual reachable addresses in the debug log file
        crate::util::logging::log("info", &format!("thin-client url: http://{}:{}/", ip, port));
    }

    println!("╟────────────────────────────────────────────────────────────────────────────╢");
    println!("║  The page is fully self-contained. No install, no cloud, works offline.    ║");
    println!("║  SSE endpoint is /events  (auto-reconnects on network hiccups).            ║");
    println!("╚════════════════════════════════════════════════════════════════════════════╝");
    println!();

    crate::util::logging::log("info", &format!("thin client server listening on port {}", port));
}

/// Minimal HTTP request handler (one thread per connection — perfectly acceptable here).
fn handle_connection(
    mut stream: TcpStream,
    clients: Arc<Mutex<Vec<MpscSender<TranscriptionUpdate>>>>,
) -> std::io::Result<()> {
    // Read request line + headers (very small buffer is enough).
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    let line = request_line.trim();
    if line.is_empty() {
        return Ok(());
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "GET" {
        write_simple_response(&mut stream, 400, "text/plain", "Bad request");
        return Ok(());
    }
    let path = parts[1];

    // Drain headers until blank line (we ignore them for this ultra-minimal server).
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
    }

    match path {
        "/" | "/subtitles" | "/subtitles.html" | "/index.html" => {
            let body = SUBTITLES_HTML;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-cache\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes())?;
            stream.write_all(body.as_bytes())?;
        }

        "/events" | "/sse" | "/stream" => {
            // SSE upgrade — keep connection open forever (or until client drops).
            let headers = "HTTP/1.1 200 OK\r\n\
                           Content-Type: text/event-stream\r\n\
                           Cache-Control: no-cache\r\n\
                           Connection: keep-alive\r\n\
                           Access-Control-Allow-Origin: *\r\n\
                           \r\n";
            stream.write_all(headers.as_bytes())?;
            stream.flush()?;

            // Create a dedicated channel for this phone/tablet.
            let (tx, rx) = mpsc::channel::<TranscriptionUpdate>();
            {
                let mut list = clients.lock().unwrap();
                list.push(tx);
            }

            // This thread now owns the stream and blocks on its private receiver.
            serve_sse_to_client(stream, rx)?;
        }

        _ => {
            write_simple_response(&mut stream, 404, "text/plain", "Not found");
        }
    }

    Ok(())
}

/// Write a dead-simple HTTP response and close.
fn write_simple_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let text = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        if status == 200 { "OK" } else { "Error" },
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(text.as_bytes());
    let _ = stream.flush();
}

/// Pump loop for one SSE client. Blocks until the client disconnects or we can't write.
fn serve_sse_to_client(
    mut stream: TcpStream,
    rx: mpsc::Receiver<TranscriptionUpdate>,
) -> std::io::Result<()> {
    for update in rx {
        let frame = format_sse_frame(&update);
        if stream.write_all(frame.as_bytes()).is_err() {
            break; // client went away (phone locked, tab closed, etc.)
        }
        let _ = stream.flush(); // critical for low-latency real-time subtitles
    }
    Ok(())
}

/// Turn an update into a valid SSE frame. We use the default "message" event.
fn format_sse_frame(update: &TranscriptionUpdate) -> String {
    // Minimal safe JSON (no full serde — we control the strings).
    let text = json_escape(&update.text);
    let json = format!(
        r#"{{"id":"{}","text":"{}","final":{},"ts":{},"speaker":"{}"}}"#,
        update.id,
        text,
        update.is_final,
        update.timestamp_ms,
        update.speaker.as_str()
    );
    // EventSource will deliver this as e.data
    format!("data: {}\n\n", json)
}

/// Extremely small JSON string escaper (sufficient for STT output).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(' '), // safe fallback
            c => out.push(c),
        }
    }
    out
}

// =============================================================================
// BEAUTIFUL HIGH-CONTRAST SUBTITLES PAGE (embedded, zero external assets)
// Target: Deaf / HoH users on phones & iPads. Giant text, zero clutter.
// =============================================================================

const SUBTITLES_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
  <meta name="apple-mobile-web-app-capable" content="yes">
  <meta name="apple-mobile-web-app-status-bar-style" content="black">
  <title>Interpres — Live Subtitles</title>
  <style>
    :root {
      --bg: #000000;
      --fg: #f8f8f8;
      --accent: #ffea00;
      --muted: #888;
      --line-border: #222;
      --partial-bg: #111;
    }
    * { box-sizing: border-box; }
    html, body {
      margin: 0; padding: 0; height: 100%; width: 100%;
      background: var(--bg); color: var(--fg);
      font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      -webkit-font-smoothing: antialiased;
      overflow: hidden;
      touch-action: manipulation;
    }
    body {
      display: flex;
      flex-direction: column;
      min-height: 100vh;
      min-height: -webkit-fill-available;
    }
    header {
      flex: 0 0 auto;
      padding: 10px 14px 8px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      border-bottom: 1px solid #1f1f1f;
      background: #000;
      z-index: 10;
    }
    .brand {
      font-size: 13px;
      font-weight: 700;
      letter-spacing: 1.5px;
      opacity: 0.7;
      user-select: none;
    }
    .status {
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 12px;
      font-weight: 600;
      color: #0f0;
    }
    .dot {
      width: 8px; height: 8px;
      background: #0f0;
      border-radius: 50%;
      box-shadow: 0 0 6px #0f0;
      animation: pulse 2s infinite ease-in-out;
    }
    .status.reconnecting { color: #ffaa00; }
    .status.reconnecting .dot { background: #ffaa00; box-shadow: 0 0 6px #ffaa00; animation: none; }

    @keyframes pulse {
      0%,100% { opacity: 1; } 50% { opacity: 0.4; }
    }

    main {
      flex: 1 1 auto;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
      padding: 14px 16px 8px;
      display: flex;
      flex-direction: column;
      gap: 11px;
      font-size: clamp(2.05rem, 7.2vw, 4.35rem);
      line-height: 1.22;
      font-weight: 700;
      scroll-behavior: smooth;
    }
    .line {
      padding: 6px 2px 7px;
      border-bottom: 1px solid var(--line-border);
      word-wrap: break-word;
      overflow-wrap: anywhere;
    }
    .line.ai {
      color: #9fd4ff;
      border-left: 4px solid #4a9eff;
      padding-left: 10px;
    }
    .line:last-child { border-bottom: none; }

    #partial {
      flex: 0 0 auto;
      min-height: 2.9em;
      padding: 13px 16px 14px;
      background: var(--partial-bg);
      border-top: 3px solid #222;
      font-size: clamp(1.85rem, 6.6vw, 3.65rem);
      font-weight: 600;
      line-height: 1.25;
      color: var(--accent);
      white-space: pre-wrap;
      word-wrap: break-word;
    }
    #partial:empty::before {
      content: "…";
      color: #444;
      font-weight: 400;
    }

    footer {
      flex: 0 0 auto;
      padding: 10px 14px 14px;
      background: #000;
      border-top: 1px solid #1f1f1f;
      display: flex;
      gap: 10px;
      align-items: center;
    }
    #clear {
      flex: 1;
      padding: 11px 20px;
      font-size: 15px;
      font-weight: 700;
      background: #1a1a1a;
      color: #ddd;
      border: 2px solid #333;
      border-radius: 6px;
      min-height: 48px;
      cursor: pointer;
      user-select: none;
      -webkit-tap-highlight-color: transparent;
    }
    #clear:active {
      background: #2a2a2a;
      transform: translateY(1px);
    }
    .hint {
      font-size: 10px;
      opacity: 0.45;
      white-space: nowrap;
      user-select: none;
    }

    .empty-hint {
      opacity: 0.25;
      font-size: clamp(1.1rem, 4vw, 1.6rem);
      font-weight: 400;
      padding: 30px 6px 10px;
      text-align: center;
      user-select: none;
    }

    /* Extra contrast boost for OLED and bright rooms */
    @media (prefers-contrast: more) {
      .line { border-bottom-color: #444; }
      #partial { border-top-color: #444; color: #fff; }
    }
  </style>
</head>
<body>
  <header>
    <div class="brand">INTERPRES</div>
    <div class="status" id="status">
      <span class="dot"></span>
      <span id="status-text">LIVE</span>
    </div>
  </header>

  <main id="transcript" aria-live="polite" aria-atomic="false"></main>

  <div id="partial" aria-live="polite" aria-atomic="true"></div>

  <footer>
    <button id="clear" aria-label="Clear all subtitles">CLEAR TRANSCRIPT</button>
    <div class="hint">local&nbsp;•&nbsp;zero&nbsp;cloud</div>
  </footer>

<script>
(function() {
  const transcript = document.getElementById('transcript');
  const partialEl = document.getElementById('partial');
  const statusEl = document.getElementById('status');
  const statusText = document.getElementById('status-text');
  const clearBtn = document.getElementById('clear');

  let eventSource = null;
  let reconnectTimer = null;
  let hasReceivedAnything = false;

  function setStatus(text, reconnecting) {
    statusText.textContent = text;
    if (reconnecting) {
      statusEl.classList.add('reconnecting');
    } else {
      statusEl.classList.remove('reconnecting');
    }
  }

  function showEmptyHint() {
    if (!hasReceivedAnything && transcript.children.length === 0) {
      const hint = document.createElement('div');
      hint.className = 'empty-hint';
      hint.id = 'empty-hint';
      hint.textContent = 'Speak — live subtitles will appear here instantly.';
      transcript.appendChild(hint);
    }
  }

  function removeEmptyHint() {
    const h = document.getElementById('empty-hint');
    if (h) h.remove();
  }

  function appendFinal(text, data) {
    if (!text || !text.trim()) return;
    removeEmptyHint();

    const line = document.createElement('div');
    line.className = 'line' + (data && data.speaker === 'ai' ? ' ai' : '');
    line.textContent = text.trim();

    transcript.appendChild(line);

    // Keep history reasonable on phones (prevents memory + scroll fatigue)
    while (transcript.children.length > 11) {
      transcript.removeChild(transcript.firstChild);
    }

    // Auto-scroll to newest
    transcript.scrollTop = transcript.scrollHeight;
    hasReceivedAnything = true;
  }

  function setPartial(text) {
    const clean = (text || '').trim();
    partialEl.textContent = clean;

    // If user is looking at history, gently bring the live line into view on significant updates
    if (clean.length > 4) {
      partialEl.scrollIntoView({ block: 'end', behavior: 'smooth' });
    }
  }

  function clearAll() {
    transcript.innerHTML = '';
    partialEl.textContent = '';
    hasReceivedAnything = false;
    showEmptyHint();
  }

  clearBtn.addEventListener('click', clearAll, { passive: true });

  // Keyboard support (desktop testing)
  document.addEventListener('keydown', (e) => {
    if (e.key.toLowerCase() === 'c' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      clearAll();
    }
    if (e.key === 'Escape') {
      clearAll();
    }
  }, { passive: false });

  // SSE connection with robust auto-reconnect (critical for phones that sleep)
  function connect() {
    if (eventSource) {
      try { eventSource.close(); } catch (_) {}
    }

    setStatus('CONNECTING…', true);

    eventSource = new EventSource('/events');

    eventSource.onopen = () => {
      setStatus('LIVE', false);
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
    };

    eventSource.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        if (data.final) {
          appendFinal(data.text, data);
          // Clear the live partial area when a sentence is committed
          if (partialEl.textContent.trim() === data.text.trim()) {
            partialEl.textContent = '';
          }
        } else {
          setPartial(data.text);
        }
        hasReceivedAnything = true;
      } catch (err) {
        // Ignore malformed frames (defensive)
      }
    };

    eventSource.onerror = () => {
      setStatus('RECONNECTING…', true);
      try { eventSource.close(); } catch (_) {}
      eventSource = null;

      // Exponential-ish backoff, capped
      if (!reconnectTimer) {
        reconnectTimer = setTimeout(() => {
          reconnectTimer = null;
          connect();
        }, 1400);
      }
    };
  }

  // Boot
  showEmptyHint();
  connect();

  // Helpful message if the user somehow opened the HTML file directly (no server)
  setTimeout(() => {
    if (!hasReceivedAnything && transcript.children.length <= 1) {
      // Only show if still looking empty after a while
    }
  }, 6500);

  // Expose a tiny debug handle (power users / dev)
  window.InterpresSubtitles = { clear: clearAll, reconnect: connect };
})();
</script>
</body>
</html>
"#;