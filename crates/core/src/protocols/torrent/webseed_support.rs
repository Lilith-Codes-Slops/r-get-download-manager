//! Detect declarations in original local metainfo, without enabling HTTP peers.
//! Do not log or persist URLs: they may contain credentials or signed tokens.
use serde::{de::{SeqAccess, Visitor}, Deserialize, Deserializer};
use std::fmt;

#[derive(Debug, Default)]
struct Seeds { count: u64 }

struct SeedBytes;
impl<'de> Deserialize<'de> for SeedBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = SeedBytes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a webseed URL byte string") }
            fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<SeedBytes, E> { Ok(SeedBytes) }
            fn visit_borrowed_bytes<E: serde::de::Error>(self, _: &'de [u8]) -> Result<SeedBytes, E> { Ok(SeedBytes) }
        }
        deserializer.deserialize_bytes(BytesVisitor)
    }
}

impl<'de> Deserialize<'de> for Seeds {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SeedsVisitor;
        impl<'de> Visitor<'de> for SeedsVisitor {
            type Value = Seeds;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a webseed byte string or list") }
            fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<Seeds, E> { Ok(Seeds { count: 1 }) }
            fn visit_borrowed_bytes<E: serde::de::Error>(self, _: &'de [u8]) -> Result<Seeds, E> { Ok(Seeds { count: 1 }) }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Seeds, A::Error> {
                let mut count = 0u64;
                while seq.next_element::<SeedBytes>()?.is_some() { count = count.saturating_add(1); }
                Ok(Seeds { count })
            }
        }
        deserializer.deserialize_any(SeedsVisitor)
    }
}

#[derive(Deserialize)]
struct Metainfo {
    #[serde(rename = "url-list", default)] url_list: Seeds,
    #[serde(rename = "httpseeds", default)] httpseeds: Seeds,
    // Other keys, including info and trackers, are ignored by serde.
}

/// None means the declaration could not be inspected; Some(0) means absent or empty.
pub fn declaration_count(bytes: &[u8]) -> Option<u64> {
    // Match the local metainfo ingress limit even if this function is called separately.
    if bytes.len() > 8 * 1024 * 1024 { return None; }
    let meta: Metainfo = librqbit_bencode::from_bytes(bytes).ok()?;
    Some(meta.url_list.count.saturating_add(meta.httpseeds.count))
}

pub fn log_metainfo(bytes: &[u8]) {
    match declaration_count(bytes) {
        Some(count) if count > 0 => tracing::info!(count, "webseed declarations detected; engine support unavailable, continuing P2P only"),
        Some(_) => tracing::debug!("no webseed declarations in local metainfo; P2P only"),
        None => tracing::warn!("could not inspect webseed declarations; continuing P2P only"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_single_and_list_forms_without_using_urls() {
        assert_eq!(declaration_count(b"d8:url-list8:http://xe"), Some(1));
        assert_eq!(declaration_count(b"d9:httpseedsl1:a1:be8:url-listl1:cee"), Some(3));
        assert_eq!(declaration_count(b"d4:infodee"), Some(0));
        assert_eq!(declaration_count(b"d8:url-listi1ee"), None);
        assert_eq!(declaration_count(b"not bencode"), None);
    }
}
