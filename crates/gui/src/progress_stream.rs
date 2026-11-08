//! Optional stream; older daemons fall back to the existing queue polling.
use crate::ipc_bridge_worker::BridgeMessage;
use std::{sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::Sender}, time::Duration};

#[cfg(target_os = "linux")]
pub(super) fn run(tx: Sender<BridgeMessage>, ctx: eframe::egui::Context, active: Arc<AtomicBool>) {
    use downloader_ipc::{ClientRequest, DaemonEvent, EndpointPathResolver, FrameDecoder, FrameEncoder};
    use std::os::unix::net::UnixStream;
    loop {
        let _result = (|| -> Result<(), String> {
            let endpoint = EndpointPathResolver::resolve().map_err(|e| e.to_string())?;
            let mut stream = UnixStream::connect(endpoint).map_err(|e| e.to_string())?;
            stream.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
            stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            FrameEncoder::write_frame(&mut stream, &ClientRequest::SubscribeProgress).map_err(|e| e.to_string())?;
            let first: DaemonEvent = FrameDecoder::read_frame(&mut stream).map_err(|e| e.to_string())?;
            if !matches!(first, DaemonEvent::ProgressReset) { return Err("progress stream unsupported".into()); }
            if tx.send(BridgeMessage::ProgressEvent(first)).is_err() { return Ok(()); }
            active.store(true, Ordering::Release); ctx.request_repaint();
            loop {
                let event: DaemonEvent = FrameDecoder::read_frame(&mut stream).map_err(|e| e.to_string())?;
                if matches!(event, DaemonEvent::ProgressHeartbeat) { continue; }
                if !matches!(event, DaemonEvent::ProgressUpdate { .. } | DaemonEvent::ProgressRemoved { .. }) { return Err("invalid progress event".into()); }
                if tx.send(BridgeMessage::ProgressEvent(event)).is_err() { return Ok(()); }
                ctx.request_repaint();
            }
        })();
        active.store(false, Ordering::Release);
        if tx.send(BridgeMessage::ProgressDisconnected).is_err() { return; }
        ctx.request_repaint();
        std::thread::sleep(Duration::from_secs(5));
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn run(_tx: Sender<BridgeMessage>, _ctx: eframe::egui::Context, _active: Arc<AtomicBool>) {}
