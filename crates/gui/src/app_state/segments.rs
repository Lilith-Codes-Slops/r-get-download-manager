//! Apply full layouts and per-segment deltas from the live stream.
use downloader_ipc::{DaemonEvent, DownloadInfo, SegmentProgress};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct LiveSegments(HashMap<String, (String, Vec<SegmentProgress>)>);
impl LiveSegments {
    pub fn clear(&mut self) { self.0.clear(); }
    pub fn apply(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::SegmentsReset => self.clear(),
            DaemonEvent::SegmentsSnapshot { task_id, generation, segments } => { self.0.insert(task_id, (generation, segments)); }
            DaemonEvent::SegmentsRemoved { task_id, generation } => {
                if self.0.get(&task_id).is_some_and(|(current, _)| current == &generation) { self.0.remove(&task_id); }
            }
            DaemonEvent::SegmentUpdate { task_id, segment_id, start_byte, end_byte, downloaded, generation: Some(generation), status: Some(status), .. } => {
                if let Some((current, segments)) = self.0.get_mut(&task_id) {
                    if current != &generation { return; }
                    if let Some(segment) = segments.iter_mut().find(|s| s.segment_id.to_string() == segment_id && s.start_byte == start_byte && s.end_byte == end_byte) {
                        segment.downloaded = downloaded; segment.status = status;
                    }
                }
            }
            _ => {}
        }
    }
    pub fn summary(&self, task: &DownloadInfo) -> Option<(usize, u64, u64)> {
        if !matches!(task.state.as_str(), "downloading" | "paused" | "pausing" | "resuming") { return None; }
        let (generation, segments) = self.0.get(&task.url)?;
        if task.partial_name.as_deref() != Some(generation.as_str()) || segments.is_empty() { return None; }
        let mut bytes = 0u64; let mut total = 0u64;
        for s in segments {
            let length = s.end_byte.checked_sub(s.start_byte)?;
            if s.downloaded > length { return None; }
            bytes = bytes.checked_add(s.downloaded)?; total = total.checked_add(length)?;
        }
        if total == 0 || task.total != Some(total) { return None; }
        Some((segments.len(), bytes, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> DownloadInfo {
        serde_json::from_str(r#"{"url":"task","state":"downloading","filename":null,"downloaded":0,"total":20,"speed_bps":0,"final_path":null,"partial_name":"attempt"}"#).unwrap()
    }
    fn segment(id: u16, start: u64, downloaded: u64) -> SegmentProgress {
        SegmentProgress { segment_id: id, start_byte: start, end_byte: start + 10, downloaded, status: "active".into() }
    }
    #[test]
    fn snapshots_deltas_and_generation_lifecycle() {
        let mut live = LiveSegments::default();
        live.apply(DaemonEvent::SegmentsSnapshot { task_id: "task".into(), generation: "attempt".into(), segments: vec![segment(0, 0, 2), segment(1, 10, 1)] });
        assert_eq!(live.summary(&task()), Some((2, 3, 20)));
        live.apply(DaemonEvent::SegmentUpdate { task_id: "task".into(), segment_id: "0".into(), start_byte: 0, end_byte: 10,
            downloaded: 6, mirror_id: None, generation: Some("attempt".into()), status: Some("active".into()) });
        assert_eq!(live.summary(&task()), Some((2, 7, 20)));
        live.apply(DaemonEvent::SegmentsRemoved { task_id: "task".into(), generation: "old".into() });
        assert_eq!(live.summary(&task()), Some((2, 7, 20)));
        let mut other = task(); other.partial_name = Some("retry".into());
        assert_eq!(live.summary(&other), None);
        live.apply(DaemonEvent::SegmentsReset);
        assert_eq!(live.summary(&task()), None);
    }
}
