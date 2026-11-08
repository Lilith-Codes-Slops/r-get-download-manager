use downloader_ipc::{ClientRequest, DaemonEvent, DaemonAutospawner, EndpointPathResolver, FrameDecoder, FrameEncoder};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

pub enum BridgeMessage {
    Event(DaemonEvent),
    ConnectionError(String),
    SegmentEvent(DaemonEvent),
    SegmentsDisconnected,
    PolicyEvent(DaemonEvent),
    PoliciesDisconnected,
}

pub struct IpcBridgeWorker {
    pub requests: Sender<ClientRequest>,
    pub updates: Receiver<BridgeMessage>,
}

impl IpcBridgeWorker {
    pub fn start(ctx: eframe::egui::Context) -> Self {
        let (requests, rx) = mpsc::channel();
        let (tx, updates) = mpsc::channel();
        let segment_tx = tx.clone();
        let segment_ctx = ctx.clone();
        std::thread::spawn(move || segment_stream(segment_tx, segment_ctx, false));
        let policy_tx = tx.clone();
        std::thread::spawn(move || segment_stream(policy_tx, ctx, true));
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(e) => { let _ = tx.send(BridgeMessage::ConnectionError(e.to_string())); return; }
            };
            let daemon = std::env::current_exe().ok()
                .and_then(|p| p.parent().map(|dir| dir.join(if cfg!(windows) { "downloader-core.exe" } else { "downloader-core" })))
                .filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("downloader-core"));
            let spawner = DaemonAutospawner::new(daemon);
            let mut last_connect = Instant::now() - Duration::from_secs(10);
            let mut connected = false;
            loop {
                if !connected && last_connect.elapsed() >= Duration::from_secs(5) {
                    last_connect = Instant::now();
                    match runtime.block_on(spawner.ensure_running()) {
                        Ok(()) => connected = true,
                        Err(e) => { let _ = tx.send(BridgeMessage::ConnectionError(e.to_string())); }
                    }
                }
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(req) => {
                        if !connected {
                            let _ = tx.send(BridgeMessage::ConnectionError("Daemon is not connected; please try again.".into()));
                            continue;
                        }
                        match roundtrip(&req) {
                            Ok(event) => { let _ = tx.send(BridgeMessage::Event(event)); }
                            Err(e) => { connected = false; let _ = tx.send(BridgeMessage::ConnectionError(e)); }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) if connected => {
                        match roundtrip(&ClientRequest::GetDetailedQueue) {
                            Ok(event) => { let _ = tx.send(BridgeMessage::Event(event)); }
                            Err(e) => { connected = false; let _ = tx.send(BridgeMessage::ConnectionError(e)); }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self { requests, updates }
    }
}

#[cfg(target_os = "linux")]
fn roundtrip(req: &ClientRequest) -> Result<DaemonEvent, String> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let endpoint = EndpointPathResolver::resolve().map_err(|e| e.to_string())?;
    let mut stream = UnixStream::connect(endpoint).map_err(|e| e.to_string())?;
    let read_timeout = if matches!(req, ClientRequest::RemoveTask { .. }) { 25 } else { 3 };
    stream.set_read_timeout(Some(Duration::from_secs(read_timeout))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    FrameEncoder::write_frame(&mut stream, req).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    FrameDecoder::read_frame(&mut stream).map_err(|e| e.to_string())
}

#[cfg(not(target_os = "linux"))]
fn roundtrip(_req: &ClientRequest) -> Result<DaemonEvent, String> {
    Err("The daemon currently supports Linux only".into())
}

#[cfg(target_os = "linux")]
fn segment_stream(tx: Sender<BridgeMessage>, ctx: eframe::egui::Context, policies: bool) {
    use std::os::unix::net::UnixStream;
    loop {
        let result = (|| -> Result<(), String> {
            let endpoint = EndpointPathResolver::resolve().map_err(|e| e.to_string())?;
            let mut stream = UnixStream::connect(endpoint).map_err(|e| e.to_string())?;
            stream.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
            stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            let request = if policies { ClientRequest::SubscribeSegmentPolicies } else { ClientRequest::SubscribeSegments };
            FrameEncoder::write_frame(&mut stream, &request).map_err(|e| e.to_string())?;
            loop {
                let event: DaemonEvent = FrameDecoder::read_frame(&mut stream).map_err(|e| e.to_string())?;
                if matches!(event, DaemonEvent::SegmentsHeartbeat) { continue; }
                let message = if policies { BridgeMessage::PolicyEvent(event) } else { BridgeMessage::SegmentEvent(event) };
                if tx.send(message).is_err() { return Ok(()); }
                ctx.request_repaint();
            }
        })();
        let _ = result;
        let message = if policies { BridgeMessage::PoliciesDisconnected } else { BridgeMessage::SegmentsDisconnected };
        if tx.send(message).is_err() { return; }
        ctx.request_repaint();
        std::thread::sleep(Duration::from_secs(5));
    }
}
#[cfg(not(target_os = "linux"))]
fn segment_stream(_tx: Sender<BridgeMessage>, _ctx: eframe::egui::Context, _policies: bool) {}
