use super::segments::SegmentMap;
use crate::daemon::{bandwidth::BandwidthLimiter, range::ResumeInfo};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

pub(super) struct Context {
    pub client: reqwest::Client,
    pub url: String,
    pub info: ResumeInfo,
    pub file: Arc<std::fs::File>,
    pub map: Arc<Mutex<SegmentMap>>,
    pub limiter: Arc<BandwidthLimiter>,
    pub stop: watch::Receiver<bool>,
    pub retire: watch::Receiver<bool>,
    pub drain: watch::Receiver<bool>,
    pub desired: Option<watch::Receiver<u8>>,
    pub owner: usize,
}

#[derive(Debug)]
pub(super) enum Exit { Idle, Retired, Rejected }

#[derive(Debug)]
enum Failure { Retired, Rejected, Fatal(String) }
impl From<String> for Failure { fn from(value: String) -> Self { Self::Fatal(value) } }

pub(super) async fn run(mut ctx: Context) -> Result<Exit, String> {
    loop {
        if *ctx.stop.borrow() { return Err("split stopped".into()); }
        if *ctx.retire.borrow() || *ctx.drain.borrow() { return Ok(Exit::Retired); }
        let work = {
            let mut map = ctx.map.lock().await;
            if *ctx.retire.borrow() || *ctx.drain.borrow() { return Ok(Exit::Retired); }
            let allow_steal = ctx.desired.as_ref().is_none_or(|receiver| *receiver.borrow() > 1);
            map.claim(ctx.owner).or_else(|| if allow_steal { map.steal(ctx.owner) } else { None })
        };
        let Some((segment, start, end)) = work else { return Ok(Exit::Idle) };
        if let Err(error) = receive_range(&mut ctx, segment, start, end).await {
            ctx.map.lock().await.release(segment, ctx.owner)?;
            return match error {
                Failure::Retired => Ok(Exit::Retired),
                Failure::Rejected => Ok(Exit::Rejected),
                Failure::Fatal(error) => Err(error),
            };
        }
    }
}

async fn receive_range(ctx: &mut Context, segment: usize, mut offset: u64, requested_end: u64) -> Result<(), Failure> {
    let request = ctx.client.get(&ctx.url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, format!("bytes={offset}-{}", requested_end - 1))
        .header(reqwest::header::IF_RANGE, ctx.info.etag.as_str());
    let started = std::time::Instant::now();
    let response = tokio::select! {
        result = request.send() => result.map_err(|e| e.to_string())?,
        _ = ctx.stop.changed() => return Err(Failure::Fatal("split stopped".into())),
        _ = ctx.retire.changed() => return Err(Failure::Retired),
    };
    // Only explicit capacity signals are retryable. Other statuses and all
    // identity/range mismatches retain the existing fail-closed behavior.
    if response.url() != &ctx.info.url { return Err(Failure::Fatal("range redirected to a different resource".into())); }
    if matches!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE) {
        ctx.map.lock().await.note_rejection(response.status().as_u16());
        return Err(Failure::Rejected);
    }
    crate::daemon::range::validate_bounded(&response, &ctx.info, offset, requested_end)?;
    ctx.map.lock().await.note_rtt(started.elapsed().as_secs_f64() * 1000.0);
    let mut body = response.bytes_stream();
    loop {
        if *ctx.stop.borrow() { return Err(Failure::Fatal("split stopped".into())); }
        if *ctx.retire.borrow() { return Err(Failure::Retired); }
        let chunk = tokio::select! {
            item = body.next() => item,
            _ = ctx.stop.changed() => return Err(Failure::Fatal("split stopped".into())),
            _ = ctx.retire.changed() => return Err(Failure::Retired),
        };
        let Some(chunk) = chunk else { return Err(Failure::Fatal("split body ended before the owned range was complete".into())) };
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.is_empty() { continue; }
        if chunk.len() as u64 > requested_end - offset { return Err(Failure::Fatal("split body exceeds its requested range".into())); }
        tokio::select! {
            _ = ctx.limiter.acquire(&ctx.url, chunk.len()) => {},
            _ = ctx.stop.changed() => return Err(Failure::Fatal("split stopped".into())),
            _ = ctx.retire.changed() => return Err(Failure::Retired),
        }
        if *ctx.stop.borrow() { return Err(Failure::Fatal("split stopped".into())); }
        if *ctx.retire.borrow() { return Err(Failure::Retired); }
        // Reserve under the map lock, then write without holding it. Stealing
        // can only transfer bytes beyond the reservation; release must wait.
        let len = ctx.map.lock().await.reserve_write(segment, ctx.owner, offset, chunk.len())?;
        let write_chunk = chunk.slice(..len);
        let file = ctx.file.clone();
        let bytes_for_write = write_chunk.clone();
        let write_offset = offset;
        let result = tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::FileExt;
            file.write_all_at(&bytes_for_write, write_offset)?;
            #[cfg(feature = "checkpoint-test-hooks")]
            super::test_hooks::block_after_write()?;
            Ok::<(), std::io::Error>(())
        }).await.map_err(|e| e.to_string()).and_then(|result| result.map_err(|e| e.to_string()));
        let mut map = ctx.map.lock().await;
        if let Err(error) = result {
            map.abort_write(segment, ctx.owner, offset)?;
            return Err(Failure::Fatal(error));
        }
        let current_end = map.commit_write(segment, ctx.owner, offset, &write_chunk)?;
        offset += len as u64;
        if offset == current_end {
            map.finish(segment, ctx.owner)?;
            // Drop the old response here. If this range was stolen, its server
            // response can still contain bytes now owned by the other worker.
            return Ok(());
        }
    }
}
