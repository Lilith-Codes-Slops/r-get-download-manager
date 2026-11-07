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
    loop {
        if *ctx.stop.borrow() { return Err("split stopped".into()); }
        let work = {
            let mut map = ctx.map.lock().await;
            map.claim(ctx.owner).or_else(|| map.steal(ctx.owner))
        };
        let Some((segment, start, end)) = work else { return Ok(()) };
        if let Err(error) = receive_range(&mut ctx, segment, start, end).await {
            let _ = ctx.map.lock().await.release(segment, ctx.owner);
            return Err(error);
        }
    }
}

async fn receive_range(ctx: &mut Context, segment: usize, mut offset: u64, requested_end: u64) -> Result<(), String> {
    let request = ctx.client.get(&ctx.url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, format!("bytes={offset}-{}", requested_end - 1))
        .header(reqwest::header::IF_RANGE, ctx.info.etag.as_str());
    let response = tokio::select! {
        result = request.send() => result.map_err(|e| e.to_string())?,
        _ = ctx.stop.changed() => return Err("split stopped".into()),
    };
    crate::daemon::range::validate_bounded(&response, &ctx.info, offset, requested_end)?;
    let mut body = response.bytes_stream();
    loop {
        if *ctx.stop.borrow() { return Err("split stopped".into()); }
        let chunk = tokio::select! {
            item = body.next() => item,
            _ = ctx.stop.changed() => return Err("split stopped".into()),
        };
        let Some(chunk) = chunk else { return Err("split body ended before the owned range was complete".into()) };
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.len() as u64 > requested_end - offset { return Err("split body exceeds its requested range".into()); }
        tokio::select! {
            _ = ctx.limiter.acquire(&ctx.url, chunk.len()) => {},
            _ = ctx.stop.changed() => return Err("split stopped".into()),
        }
        if *ctx.stop.borrow() { return Err("split stopped".into()); }
        // Holding the map lock across the write is the ownership and checkpoint
        // barrier. A steal can only shrink the unwritten portion after this write.
        let mut map = ctx.map.lock().await;
        let current_end = map.owned_end(segment, ctx.owner, offset)?;
        let len = (current_end - offset).min(chunk.len() as u64) as usize;
        let write_chunk = chunk.slice(..len);
        let file = ctx.file.clone();
        let bytes_for_write = write_chunk.clone();
        let write_offset = offset;
        tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::FileExt;
            file.write_all_at(&bytes_for_write, write_offset)
        }).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        map.advance(segment, ctx.owner, offset, &write_chunk)?;
        offset += len as u64;
        if offset == current_end {
            map.finish(segment, ctx.owner)?;
            // Drop the old response here. If this range was stolen, its server
            // response can still contain bytes now owned by the other worker.
            return Ok(());
        }
    }
}
