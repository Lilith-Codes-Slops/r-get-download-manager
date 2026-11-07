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
    pub owner: usize,
}

pub(super) async fn run(mut ctx: Context) -> Result<(), String> {
    if *ctx.stop.borrow() { return Err("split stopped".into()); }
    let (segment, mut offset, end) = ctx.map.lock().await.claim(ctx.owner).ok_or("no unowned split segment")?;
    let request = ctx.client.get(&ctx.url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, format!("bytes={offset}-{}", end - 1))
        .header(reqwest::header::IF_RANGE, ctx.info.etag.as_str());
    let response = tokio::select! {
        result = request.send() => result.map_err(|e| e.to_string())?,
        _ = ctx.stop.changed() => return Err("split stopped".into()),
    };
    crate::daemon::range::validate_bounded(&response, &ctx.info, offset, end)?;
    let mut body = response.bytes_stream();
    loop {
        if *ctx.stop.borrow() { return Err("split stopped".into()); }
        let chunk = tokio::select! {
            item = body.next() => item,
            _ = ctx.stop.changed() => return Err("split stopped".into()),
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.len() as u64 > end - offset { return Err("split body exceeds its range".into()); }
        tokio::select! {
            _ = ctx.limiter.acquire(&ctx.url, chunk.len()) => {},
            _ = ctx.stop.changed() => return Err("split stopped".into()),
        }
        if *ctx.stop.borrow() { return Err("split stopped".into()); }
        // A positional write doesn't change the shared file cursor. Await it to
        // completion even if another worker fails, before releasing ownership.
        // This lock is also the checkpoint barrier: snapshot+sync cannot race
        // a write or observe progress before its corresponding bytes exist.
        let mut map = ctx.map.lock().await;
        let file = ctx.file.clone();
        let write_chunk = chunk.clone();
        let write_offset = offset;
        tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::FileExt;
            file.write_all_at(&write_chunk, write_offset)
        }).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        map.advance(segment, ctx.owner, offset, &chunk)?;
        offset += chunk.len() as u64;
    }
    if offset != end { return Err("split body ended before the requested range was complete".into()); }
    ctx.map.lock().await.finish(segment, ctx.owner)?;
    Ok(())
}
