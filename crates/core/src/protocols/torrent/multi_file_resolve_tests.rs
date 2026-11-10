use super::{multi_file_resolve, session, task_identity::PreparedSource};
use librqbit::Session;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn metainfo(path: &[u8]) -> Vec<u8> {
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl".to_vec();
    bytes.extend_from_slice(format!("{}:", path.len()).as_bytes());
    bytes.extend_from_slice(path);
    bytes.extend_from_slice(b"eee4:name6:bundle12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&[0; 20]);
    bytes.extend_from_slice(b"ee");
    bytes
}

#[tokio::test]
async fn local_list_only_preflights_and_never_creates_payload_files() {
    let root = std::env::temp_dir().join(format!(
        "rget-multi-resolve-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let mut options = session::download_only_options();
    options.dht = None;
    let engine = Session::new_with_opts(root.clone(), options).await.unwrap();
    let bytes = metainfo(b"a");
    let hash = librqbit::torrent_from_bytes(&bytes)
        .unwrap()
        .info_hash
        .as_string();
    let result =
        multi_file_resolve::resolve(&engine, PreparedSource::Metainfo(bytes.clone()), &hash)
            .await
            .unwrap();
    assert_eq!(result.manifest.root_name(), "bundle");
    assert_eq!(result.manifest.total_bytes(), 3);
    assert_eq!(result.manifest.files()[0].components, vec!["a".to_string()]);
    assert!(fs::read_dir(&root).unwrap().next().is_none());
    assert!(multi_file_resolve::resolve(
        &engine,
        PreparedSource::Metainfo(bytes),
        "0000000000000000000000000000000000000000"
    )
    .await
    .is_err());
    let unsafe_bytes = metainfo(b"..");
    let unsafe_hash = librqbit::torrent_from_bytes(&unsafe_bytes)
        .unwrap()
        .info_hash
        .as_string();
    assert!(multi_file_resolve::resolve(
        &engine,
        PreparedSource::Metainfo(unsafe_bytes),
        &unsafe_hash
    )
    .await
    .is_err());
    let magnet = format!("magnet:?xt=urn:btih:{hash}");
    let result = multi_file_resolve::resolve(&engine, PreparedSource::Magnet(magnet), &hash).await;
    assert!(
        matches!(result, Err(super::metadata::MetadataError::Invalid(message)) if message.contains("disabled"))
    );
    assert!(fs::read_dir(&root).unwrap().next().is_none());
    engine.stop().await;
    fs::remove_dir_all(root).unwrap();
}
