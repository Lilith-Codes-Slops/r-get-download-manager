//! Non-overlapping ranges and exclusive ownership under the segment-map lock.
const MIN_SEGMENT: u64 = 1024 * 1024;
const MAX_CONNECTIONS: u8 = 32;
pub(super) const MAX_SEGMENTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pending,
    Active(usize),
    Done,
}

#[derive(Clone, Debug)]
struct Segment {
    start: u64,
    end: u64,
    downloaded: u64,
    status: Status,
    hash: blake3::Hasher,
    in_flight: Option<u64>,
    request_end: Option<u64>,
}

// Indices are stable: stealing appends a segment, never inserts/removes one.
pub(in crate::daemon) struct SegmentMap {
    segments: Vec<Segment>,
    rejected: Option<u16>,
    changes: std::sync::Arc<tokio::sync::Notify>,
    sizing: super::sizing_state::SizingState,
    policy_publisher: Option<std::sync::Weak<crate::daemon::segment_events::Publisher>>,
}

impl SegmentMap {
    pub fn new(total: u64, requested: u8) -> Result<Self, String> {
        if !(2..=MAX_CONNECTIONS).contains(&requested) {
            return Err("split connections must be between 2 and 32".into());
        }
        let count = (total / MIN_SEGMENT).min(u64::from(requested)) as usize;
        if count < 2 {
            return Err("file is too small for two minimum-sized split ranges".into());
        }
        let base = total / count as u64;
        let extra = total % count as u64;
        let mut start = 0;
        let mut segments = Vec::with_capacity(count);
        for index in 0..count {
            let length = base + if (index as u64) < extra { 1 } else { 0 };
            let end = start + length;
            segments.push(Segment {
                start,
                end,
                downloaded: 0,
                status: Status::Pending,
                hash: blake3::Hasher::new(),
                in_flight: None,
                request_end: None,
            });
            start = end;
        }
        Ok(Self {
            segments,
            rejected: None,
            changes: std::sync::Arc::new(tokio::sync::Notify::new()),
            sizing: super::sizing_state::SizingState::default(),
            policy_publisher: None,
        })
    }

    pub fn claim(&mut self, owner: usize) -> Option<(usize, u64, u64)> {
        if self
            .segments
            .iter()
            .any(|segment| segment.status == Status::Active(owner))
        {
            return None;
        }
        let (index, segment) = self
            .segments
            .iter_mut()
            .enumerate()
            .find(|(_, segment)| segment.status == Status::Pending)?;
        segment.status = Status::Active(owner);
        let work = (index, segment.start + segment.downloaded, segment.end);
        self.changes.notify_one();
        Some(work)
    }

    /// Transfer only unreserved, unwritten work; the donor retains at least 1 MiB.
    pub fn steal(&mut self, owner: usize) -> Option<(usize, u64, u64)> {
        if self.segments.len() >= MAX_SEGMENTS
            || self
                .segments
                .iter()
                .any(|s| s.status == Status::Active(owner))
        {
            return None;
        }
        let now = std::time::Instant::now();
        let before = self.sizing.snapshot(now);
        let floor = self.sizing.decide_floor(now);
        self.emit_policy_change(before, now);
        let (index, _) = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.status, Status::Active(_)))
            .map(|(i, s)| {
                (
                    i,
                    s.end
                        - s.request_end
                            .or(s.in_flight)
                            .unwrap_or(s.start + s.downloaded),
                )
            })
            .filter(|(_, remaining)| *remaining > floor && *remaining >= floor + MIN_SEGMENT)
            .max_by_key(|(_, remaining)| *remaining)?;
        let source = &mut self.segments[index];
        // A reserved write is owned by the donor until it joins; only the
        // suffix beyond that reservation can be split.
        let remaining_start = source
            .request_end
            .or(source.in_flight)
            .unwrap_or(source.start + source.downloaded);
        let old_end = source.end;
        let remaining = old_end - remaining_start;
        let target = self.sizing.snapshot(now).target_size;
        let transferred = target.min(remaining - MIN_SEGMENT);
        if transferred < floor {
            return None;
        }
        let midpoint = old_end - transferred;
        source.end = midpoint;
        let new_index = self.segments.len();
        self.segments.push(Segment {
            start: midpoint,
            end: old_end,
            downloaded: 0,
            status: Status::Active(owner),
            hash: blake3::Hasher::new(),
            in_flight: None,
            request_end: None,
        });
        self.changes.notify_one();
        Some((new_index, midpoint, old_end))
    }

    pub fn owned_end(&self, index: usize, owner: usize, offset: u64) -> Result<u64, String> {
        let segment = self.segments.get(index).ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner)
            || offset != segment.start + segment.downloaded
            || offset > segment.end
        {
            return Err("split ownership or write boundary violation".into());
        }
        Ok(segment.end)
    }
    /// Lease the next bounded request. Steals cannot take any byte in this lease.
    pub fn begin_request(
        &mut self,
        index: usize,
        owner: usize,
        offset: u64,
    ) -> Result<u64, String> {
        let end = self.owned_end(index, owner, offset)?;
        let target = self.sizing.snapshot(std::time::Instant::now()).target_size;
        let request_end = offset.saturating_add(target).min(end);
        let segment = &mut self.segments[index];
        if offset >= end || segment.request_end.is_some() || segment.in_flight.is_some() {
            return Err("invalid range request lease".into());
        }
        segment.request_end = Some(request_end);
        Ok(request_end)
    }
    pub fn finish_request(
        &mut self,
        index: usize,
        owner: usize,
        offset: u64,
    ) -> Result<bool, String> {
        let end = self.owned_end(index, owner, offset)?;
        let segment = &mut self.segments[index];
        if segment.request_end != Some(offset) || segment.in_flight.is_some() {
            return Err("range request lease mismatch".into());
        }
        segment.request_end = None;
        self.sizing.request_completed(std::time::Instant::now());
        if offset == end {
            self.finish(index, owner)?;
            return Ok(true);
        }
        Ok(false)
    }
    /// Reserve a chunk under the map lock. Steals must start beyond it, and
    /// ownership cannot be released until the write is committed or aborted.
    pub fn reserve_write(
        &mut self,
        index: usize,
        owner: usize,
        offset: u64,
        length: usize,
    ) -> Result<usize, String> {
        let end = self.owned_end(index, owner, offset)?;
        let segment = &mut self.segments[index];
        if segment.in_flight.is_some() {
            return Err("split write already in flight".into());
        }
        let end = end.min(segment.request_end.unwrap_or(end));
        let len = (end - offset).min(length as u64) as usize;
        if len == 0 {
            return Err("empty split write reservation".into());
        }
        segment.in_flight = Some(offset + len as u64);
        Ok(len)
    }
    pub fn commit_write(
        &mut self,
        index: usize,
        owner: usize,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, String> {
        let end = self.owned_end(index, owner, offset)?;
        let segment = &mut self.segments[index];
        let write_end = offset
            .checked_add(bytes.len() as u64)
            .ok_or("split write offset overflow")?;
        if segment.in_flight != Some(write_end) || bytes.is_empty() {
            return Err("split write reservation mismatch".into());
        }
        segment.hash.update(bytes);
        segment.downloaded += bytes.len() as u64;
        segment.in_flight = None;
        Ok(end)
    }
    pub fn abort_write(&mut self, index: usize, owner: usize, offset: u64) -> Result<(), String> {
        self.owned_end(index, owner, offset)?;
        let segment = &mut self.segments[index];
        if segment.in_flight.take().is_none() {
            return Err("no split write in flight".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn advance(
        &mut self,
        index: usize,
        owner: usize,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), String> {
        let end = self.owned_end(index, owner, offset)?;
        if bytes.len() as u64 > end - offset {
            return Err("split write exceeds owned range".into());
        }
        let segment = &mut self.segments[index];
        if segment.in_flight.is_some() {
            return Err("split write already in flight".into());
        }
        segment.hash.update(bytes);
        segment.downloaded += bytes.len() as u64;
        Ok(())
    }
    pub fn finish(&mut self, index: usize, owner: usize) -> Result<(), String> {
        let segment = self
            .segments
            .get_mut(index)
            .ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner)
            || segment.in_flight.is_some()
            || segment.request_end.is_some()
            || segment.downloaded != segment.end - segment.start
        {
            return Err("cannot finish an unowned or incomplete segment".into());
        }
        segment.status = Status::Done;
        self.changes.notify_one();
        Ok(())
    }
    pub fn release(&mut self, index: usize, owner: usize) -> Result<(), String> {
        let segment = self
            .segments
            .get_mut(index)
            .ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner) || segment.in_flight.is_some() {
            return Err("cannot release an unowned or in-flight segment".into());
        }
        segment.request_end = None;
        segment.status = if segment.downloaded == segment.end - segment.start {
            Status::Done
        } else {
            Status::Pending
        };
        self.changes.notify_one();
        Ok(())
    }
    pub fn progress(&self) -> Vec<downloader_ipc::SegmentProgress> {
        self.segments
            .iter()
            .enumerate()
            .map(|(index, segment)| downloader_ipc::SegmentProgress {
                segment_id: index as u16,
                start_byte: segment.start,
                end_byte: segment.end,
                downloaded: segment.downloaded,
                status: match segment.status {
                    Status::Pending => "pending",
                    Status::Active(_) => "active",
                    Status::Done => "done",
                }
                .into(),
            })
            .collect()
    }
    pub fn records(&self) -> Vec<super::checkpoint::SegmentRecord> {
        let mut records: Vec<_> = self
            .segments
            .iter()
            .map(|segment| super::checkpoint::SegmentRecord {
                start: segment.start,
                end: segment.end,
                downloaded: segment.downloaded,
                hash: segment.hash.finalize().to_hex().to_string(),
            })
            .collect();
        records.sort_by_key(|record| record.start);
        records
    }
    pub fn restore(
        records: &[super::checkpoint::SegmentRecord],
        hashes: Vec<blake3::Hasher>,
    ) -> Result<Self, String> {
        if records.len() != hashes.len() {
            return Err("segment hash count mismatch".into());
        }
        let segments = records
            .iter()
            .zip(hashes)
            .map(|(record, hash)| Segment {
                start: record.start,
                end: record.end,
                downloaded: record.downloaded,
                status: if record.downloaded == record.end - record.start {
                    Status::Done
                } else {
                    Status::Pending
                },
                hash,
                in_flight: None,
                request_end: None,
            })
            .collect();
        Ok(Self {
            segments,
            rejected: None,
            changes: std::sync::Arc::new(tokio::sync::Notify::new()),
            sizing: super::sizing_state::SizingState::default(),
            policy_publisher: None,
        })
    }
    pub fn stealable_slots(&self) -> usize {
        let now = std::time::Instant::now();
        let floor = self.sizing.current_floor(now);
        if self.sizing.snapshot(now).target_size < floor {
            return 0;
        }
        let available = self
            .segments
            .iter()
            .filter(|s| matches!(s.status, Status::Active(_)))
            .map(|s| {
                (s.end
                    - s.request_end
                        .or(s.in_flight)
                        .unwrap_or(s.start + s.downloaded))
                .saturating_sub(MIN_SEGMENT)
                    / floor
            })
            .map(|count| count.min(MAX_SEGMENTS as u64) as usize)
            .sum::<usize>();
        available.min(MAX_SEGMENTS.saturating_sub(self.segments.len()))
    }
    pub fn active_count(&self) -> u8 {
        self.segments
            .iter()
            .filter(|s| matches!(s.status, Status::Active(_)))
            .count() as u8
    }
    pub fn segment_count(&self) -> u16 {
        self.segments.len() as u16
    }
    pub fn changes(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.changes.clone()
    }
    pub fn attach_policy_publisher(
        &mut self,
        publisher: std::sync::Weak<crate::daemon::segment_events::Publisher>,
    ) {
        self.policy_publisher = Some(publisher);
    }
    pub(super) fn seed_policy(&mut self, remembered: super::host_memory::Remembered) {
        self.sizing = super::sizing_state::SizingState::remembered(remembered);
    }
    pub(super) fn reset_stability(&mut self) {
        self.sizing.reset_stability();
    }
    pub(super) fn good_memory(
        &self,
        now: std::time::Instant,
    ) -> Option<super::host_memory::Observation> {
        self.sizing.good_memory(now)
    }
    pub fn policy(&self) -> super::sizing_state::PolicySnapshot {
        self.sizing.snapshot(std::time::Instant::now())
    }
    fn emit_policy_change(
        &self,
        before: super::sizing_state::PolicySnapshot,
        now: std::time::Instant,
    ) {
        let after = self.sizing.snapshot(now);
        if before != after {
            if let Some(publisher) = self.policy_publisher.as_ref().and_then(|p| p.upgrade()) {
                publisher.publish_policy(after, false);
            }
            self.changes.notify_one();
        }
    }
    fn observe_policy(&mut self, goodput: Option<f64>, rtt: Option<f64>) {
        let now = std::time::Instant::now();
        let before = self.sizing.snapshot(now);
        self.sizing.observe(goodput, rtt, now);
        self.emit_policy_change(before, now);
    }
    pub fn note_rtt(&mut self, rtt_ms: f64) {
        self.observe_policy(None, Some(rtt_ms));
    }
    pub fn note_goodput(&mut self, bps: f64) {
        self.observe_policy(Some(bps), None);
    }
    pub fn note_rejection(&mut self, status: u16) {
        let now = std::time::Instant::now();
        let before = self.sizing.snapshot(now);
        self.rejected = Some(status);
        self.sizing.reject(now);
        self.emit_policy_change(before, now);
        self.changes.notify_one();
    }
    pub fn was_rejected(&self) -> bool {
        self.rejected.is_some()
    }
    pub fn rejection_status(&self) -> Option<u16> {
        self.rejected
    }
    pub fn pending_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| segment.status == Status::Pending)
            .count()
    }
    pub fn is_complete(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| segment.status == Status::Done)
    }
}

#[cfg(test)]
#[path = "segment_map_tests.rs"]
mod tests;
