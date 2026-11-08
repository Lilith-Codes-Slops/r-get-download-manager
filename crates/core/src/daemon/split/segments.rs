//! Non-overlapping ranges and exclusive ownership under the segment-map lock.
const MIN_SEGMENT: u64 = 1024 * 1024;
const MAX_CONNECTIONS: u8 = 32;
pub(super) const MAX_SEGMENTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status { Pending, Active(usize), Done }

#[derive(Clone, Debug)]
struct Segment { start: u64, end: u64, downloaded: u64, status: Status, hash: blake3::Hasher, in_flight: Option<u64> }

// Indices are stable: stealing appends a segment, never inserts/removes one.
pub(super) struct SegmentMap { segments: Vec<Segment>, rejected: bool }

impl SegmentMap {
    pub fn new(total: u64, requested: u8) -> Result<Self, String> {
        if !(2..=MAX_CONNECTIONS).contains(&requested) { return Err("split connections must be between 2 and 32".into()); }
        let count = (total / MIN_SEGMENT).min(u64::from(requested)) as usize;
        if count < 2 { return Err("file is too small for two minimum-sized split ranges".into()); }
        let base = total / count as u64;
        let extra = total % count as u64;
        let mut start = 0;
        let mut segments = Vec::with_capacity(count);
        for index in 0..count {
            let length = base + if (index as u64) < extra { 1 } else { 0 };
            let end = start + length;
            segments.push(Segment { start, end, downloaded: 0, status: Status::Pending, hash: blake3::Hasher::new(), in_flight: None });
            start = end;
        }
        Ok(Self { segments, rejected: false })
    }

    pub fn claim(&mut self, owner: usize) -> Option<(usize, u64, u64)> {
        if self.segments.iter().any(|segment| segment.status == Status::Active(owner)) { return None; }
        let (index, segment) = self.segments.iter_mut().enumerate().find(|(_, segment)| segment.status == Status::Pending)?;
        segment.status = Status::Active(owner);
        Some((index, segment.start + segment.downloaded, segment.end))
    }

    /// Only steal unwritten work. Both halves must retain at least 1 MiB.
    pub fn steal(&mut self, owner: usize) -> Option<(usize, u64, u64)> {
        if self.segments.len() >= MAX_SEGMENTS || self.segments.iter().any(|s| s.status == Status::Active(owner)) { return None; }
        let (index, _) = self.segments.iter().enumerate()
            .filter(|(_, s)| matches!(s.status, Status::Active(_)))
            .map(|(i, s)| (i, s.end - s.in_flight.unwrap_or(s.start + s.downloaded)))
            .filter(|(_, remaining)| *remaining >= 2 * MIN_SEGMENT)
            .max_by_key(|(_, remaining)| *remaining)?;
        let source = &mut self.segments[index];
        // A reserved write is owned by the donor until it joins; only the
        // suffix beyond that reservation can be split.
        let remaining_start = source.in_flight.unwrap_or(source.start + source.downloaded);
        let old_end = source.end;
        let midpoint = remaining_start + (old_end - remaining_start) / 2;
        source.end = midpoint;
        let new_index = self.segments.len();
        self.segments.push(Segment { start: midpoint, end: old_end, downloaded: 0,
            status: Status::Active(owner), hash: blake3::Hasher::new(), in_flight: None });
        Some((new_index, midpoint, old_end))
    }

    pub fn owned_end(&self, index: usize, owner: usize, offset: u64) -> Result<u64, String> {
        let segment = self.segments.get(index).ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner) || offset != segment.start + segment.downloaded || offset > segment.end {
            return Err("split ownership or write boundary violation".into());
        }
        Ok(segment.end)
    }
    /// Reserve a chunk under the map lock. Steals must start beyond it, and
    /// ownership cannot be released until the write is committed or aborted.
    pub fn reserve_write(&mut self, index: usize, owner: usize, offset: u64, length: usize) -> Result<usize, String> {
        let end = self.owned_end(index, owner, offset)?;
        let segment = &mut self.segments[index];
        if segment.in_flight.is_some() { return Err("split write already in flight".into()); }
        let len = (end - offset).min(length as u64) as usize;
        if len == 0 { return Err("empty split write reservation".into()); }
        segment.in_flight = Some(offset + len as u64);
        Ok(len)
    }
    pub fn commit_write(&mut self, index: usize, owner: usize, offset: u64, bytes: &[u8]) -> Result<u64, String> {
        let end = self.owned_end(index, owner, offset)?;
        let segment = &mut self.segments[index];
        let write_end = offset.checked_add(bytes.len() as u64).ok_or("split write offset overflow")?;
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
        if segment.in_flight.take().is_none() { return Err("no split write in flight".into()); }
        Ok(())
    }
    #[cfg(test)]
    pub fn advance(&mut self, index: usize, owner: usize, offset: u64, bytes: &[u8]) -> Result<(), String> {
        let end = self.owned_end(index, owner, offset)?;
        if bytes.len() as u64 > end - offset { return Err("split write exceeds owned range".into()); }
        let segment = &mut self.segments[index];
        if segment.in_flight.is_some() { return Err("split write already in flight".into()); }
        segment.hash.update(bytes);
        segment.downloaded += bytes.len() as u64;
        Ok(())
    }
    pub fn finish(&mut self, index: usize, owner: usize) -> Result<(), String> {
        let segment = self.segments.get_mut(index).ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner) || segment.in_flight.is_some() || segment.downloaded != segment.end - segment.start {
            return Err("cannot finish an unowned or incomplete segment".into());
        }
        segment.status = Status::Done;
        Ok(())
    }
    pub fn release(&mut self, index: usize, owner: usize) -> Result<(), String> {
        let segment = self.segments.get_mut(index).ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner) || segment.in_flight.is_some() { return Err("cannot release an unowned or in-flight segment".into()); }
        segment.status = if segment.downloaded == segment.end - segment.start { Status::Done } else { Status::Pending };
        Ok(())
    }
    pub fn records(&self) -> Vec<super::checkpoint::SegmentRecord> {
        let mut records: Vec<_> = self.segments.iter().map(|segment| super::checkpoint::SegmentRecord {
            start: segment.start, end: segment.end, downloaded: segment.downloaded,
            hash: segment.hash.finalize().to_hex().to_string(),
        }).collect();
        records.sort_by_key(|record| record.start);
        records
    }
    pub fn restore(records: &[super::checkpoint::SegmentRecord], hashes: Vec<blake3::Hasher>) -> Result<Self, String> {
        if records.len() != hashes.len() { return Err("segment hash count mismatch".into()); }
        let segments = records.iter().zip(hashes).map(|(record, hash)| Segment {
            start: record.start, end: record.end, downloaded: record.downloaded,
            status: if record.downloaded == record.end - record.start { Status::Done } else { Status::Pending }, hash, in_flight: None,
        }).collect();
        Ok(Self { segments, rejected: false })
    }
    pub fn active_count(&self) -> u8 { self.segments.iter().filter(|s| matches!(s.status, Status::Active(_))).count() as u8 }
    pub fn segment_count(&self) -> u16 { self.segments.len() as u16 }
    pub fn note_rejection(&mut self) { self.rejected = true; }
    pub fn was_rejected(&self) -> bool { self.rejected }
    pub fn pending_count(&self) -> usize { self.segments.iter().filter(|segment| segment.status == Status::Pending).count() }
    pub fn is_complete(&self) -> bool { self.segments.iter().all(|segment| segment.status == Status::Done) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partition_covers_every_byte_and_obeys_minimum() {
        for total in [2 * MIN_SEGMENT, 3 * MIN_SEGMENT + 7, 17 * MIN_SEGMENT + 1] {
            for count in 2..=32 {
                let map = SegmentMap::new(total, count).unwrap();
                let mut next = 0;
                for segment in map.segments {
                    assert_eq!(segment.start, next);
                    assert!(segment.end - segment.start >= MIN_SEGMENT);
                    next = segment.end;
                }
                assert_eq!(next, total);
            }
        }
        assert!(SegmentMap::new(MIN_SEGMENT, 8).is_err());
        assert!(SegmentMap::new(10 * MIN_SEGMENT, 1).is_err());
        assert!(SegmentMap::new(40 * MIN_SEGMENT, 33).is_err());
    }
    #[test]
    fn ownership_and_offsets_are_checked() {
        let mut map = SegmentMap::new(2 * MIN_SEGMENT, 2).unwrap();
        let (index, start, end) = map.claim(7).unwrap();
        assert!(map.claim(7).is_none());
        assert!(map.advance(index, 8, start, b"a").is_err());
        assert!(map.advance(index, 7, start + 1, b"a").is_err());
        assert!(map.advance(index, 7, start, &vec![0; (end - start + 1) as usize]).is_err());
        assert!(map.finish(index, 7).is_err());
        map.advance(index, 7, start, &vec![0; (end - start) as usize]).unwrap();
        map.finish(index, 7).unwrap();
        assert!(!map.is_complete());
    }
    #[test]
    fn steal_transfers_only_unwritten_bytes_and_release_preserves_progress() {
        let mut map = SegmentMap::new(8 * MIN_SEGMENT, 2).unwrap();
        let (first, first_start, _) = map.claim(0).unwrap();
        let (second, second_start, second_end) = map.claim(1).unwrap();
        map.advance(second, 1, second_start, b"abc").unwrap();
        assert!(map.steal(0).is_none()); // an owner cannot hold two ranges
        map.release(first, 0).unwrap();
        let (stolen, midpoint, end) = map.steal(0).unwrap();
        assert_eq!(end, second_end);
        assert_eq!(map.owned_end(second, 1, second_start + 3).unwrap(), midpoint);
        assert!(midpoint - (second_start + 3) >= MIN_SEGMENT);
        assert!(end - midpoint >= MIN_SEGMENT);
        assert!(map.advance(second, 1, midpoint, b"wrong owner").is_err());
        let past_end = vec![0; (midpoint - (second_start + 3) + 1) as usize];
        assert!(map.advance(second, 1, second_start + 3, &past_end).is_err());
        map.advance(stolen, 0, midpoint, b"x").unwrap();
        map.release(stolen, 0).unwrap();
        let (reclaimed, resume, _) = map.claim(2).unwrap();
        assert_eq!(reclaimed, first);
        assert_eq!(resume, first_start);
        map.release(reclaimed, 2).unwrap();
        let records = map.records();
        assert_eq!(records.first().unwrap().start, 0);
        for pair in records.windows(2) { assert_eq!(pair[0].end, pair[1].start); }
        assert_eq!(records.last().unwrap().end, 8 * MIN_SEGMENT);
    }
    #[test]
    fn reservations_exclude_uncommitted_bytes_and_protect_inflight_writes() {
        let mut map = SegmentMap::new(12 * MIN_SEGMENT, 2).unwrap();
        let (first, start, _) = map.claim(0).unwrap();
        let (second, other, _) = map.claim(1).unwrap();
        assert_eq!(map.reserve_write(first, 0, start, 4).unwrap(), 4);
        assert_eq!(map.reserve_write(second, 1, other, 3).unwrap(), 3);
        assert_eq!(map.records()[0].downloaded, 0);
        assert!(map.release(first, 0).is_err());
        assert!(map.commit_write(first, 0, start, b"bad").is_err());
        let (stolen, midpoint, _) = map.steal(2).unwrap();
        assert!(midpoint >= other + 3);
        assert!(map.release(stolen, 2).is_ok());
        map.commit_write(first, 0, start, b"abcd").unwrap();
        map.abort_write(second, 1, other).unwrap();
        assert_eq!(map.records()[0].downloaded, 4);
        assert!(map.release(second, 1).is_ok());
    }
    #[test]
    fn never_steals_below_minimum_floor() {
        let mut map = SegmentMap::new(4 * MIN_SEGMENT, 2).unwrap();
        let (idx, start, _) = map.claim(0).unwrap();
        map.advance(idx, 0, start, b"x").unwrap();
        assert!(map.steal(1).is_none());
    }
}
