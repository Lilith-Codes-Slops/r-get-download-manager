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

    #[test]
    fn leases_and_steals_follow_target_without_altering_inflight_work() {
        let mut map = SegmentMap::new(128 * MIN_SEGMENT, 2).unwrap();
        let (idx, start, _) = map.claim(0).unwrap();
        let first_end = map.begin_request(idx, 0, start).unwrap();
        assert_eq!(first_end - start, 8 * MIN_SEGMENT);
        map.note_rtt(10.0);
        map.note_goodput(2.0 * MIN_SEGMENT as f64);
        let (_, grown_start, grown_end) = map.steal(1).unwrap();
        assert_eq!(grown_end - grown_start, 12 * MIN_SEGMENT);
        assert!(grown_start >= first_end);
        map.note_rejection(429);
        let (_, small_start, small_end) = map.steal(2).unwrap();
        assert_eq!(small_end - small_start, 6 * MIN_SEGMENT);
        assert!(small_start >= first_end);
        assert!(map.finish_request(idx, 0, start).is_err());
        assert_eq!(map.segments[idx].request_end, Some(first_end));
        map.release(idx, 0).unwrap();
        let (idx, offset, _) = map.claim(3).unwrap();
        assert_eq!(map.begin_request(idx, 3, offset).unwrap() - offset, 6 * MIN_SEGMENT);
    }

    #[test]
    fn short_remainders_are_leased_and_large_files_keep_compact_records() {
        let mut map = SegmentMap::new(8 * 1024 * MIN_SEGMENT, 2).unwrap();
        let (idx, start, _) = map.claim(0).unwrap();
        assert_eq!(map.begin_request(idx, 0, start).unwrap(), start + 8 * MIN_SEGMENT);
        assert_eq!(map.records().len(), 2);
        map.release(idx, 0).unwrap();
        let mut small = SegmentMap::new(3 * MIN_SEGMENT, 2).unwrap();
        let (idx, start, end) = small.claim(0).unwrap();
        assert_eq!(small.begin_request(idx, 0, start).unwrap(), end);
        assert!(small.reserve_write(idx, 0, start, (end - start + 1) as usize).unwrap() as u64 <= end - start);
        small.abort_write(idx, 0, start).unwrap();
        small.release(idx, 0).unwrap();
    }
