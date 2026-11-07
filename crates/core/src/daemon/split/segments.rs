//! Fixed non-overlapping ranges with exclusive worker ownership.
const MIN_SEGMENT: u64 = 1024 * 1024;
const MAX_CONNECTIONS: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status { Pending, Active(usize), Done }

#[derive(Clone, Debug)]
struct Segment { start: u64, end: u64, downloaded: u64, status: Status }

pub(super) struct SegmentMap { segments: Vec<Segment> }

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
            segments.push(Segment { start, end, downloaded: 0, status: Status::Pending });
            start = end;
        }
        Ok(Self { segments })
    }

    pub fn len(&self) -> usize { self.segments.len() }
    pub fn claim(&mut self, owner: usize) -> Option<(usize, u64, u64)> {
        if self.segments.iter().any(|segment| segment.status == Status::Active(owner)) { return None; }
        let (index, segment) = self.segments.iter_mut().enumerate().find(|(_, segment)| segment.status == Status::Pending)?;
        segment.status = Status::Active(owner);
        Some((index, segment.start + segment.downloaded, segment.end))
    }
    pub fn advance(&mut self, index: usize, owner: usize, offset: u64, bytes: u64) -> Result<(), String> {
        let segment = self.segments.get_mut(index).ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner) || offset != segment.start + segment.downloaded
            || bytes > segment.end - offset {
            return Err("split ownership or write boundary violation".into());
        }
        segment.downloaded += bytes;
        Ok(())
    }
    pub fn finish(&mut self, index: usize, owner: usize) -> Result<(), String> {
        let segment = self.segments.get_mut(index).ok_or("unknown split segment")?;
        if segment.status != Status::Active(owner) || segment.downloaded != segment.end - segment.start {
            return Err("cannot finish an unowned or incomplete segment".into());
        }
        segment.status = Status::Done;
        Ok(())
    }
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
        assert!(map.advance(index, 8, start, 1).is_err());
        assert!(map.advance(index, 7, start + 1, 1).is_err());
        assert!(map.advance(index, 7, start, end - start + 1).is_err());
        assert!(map.finish(index, 7).is_err());
        map.advance(index, 7, start, end - start).unwrap();
        map.finish(index, 7).unwrap();
        assert!(!map.is_complete());
    }
}
