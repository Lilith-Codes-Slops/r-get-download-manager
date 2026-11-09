#![cfg(all(target_os = "linux", feature = "torrent"))]
//! Local librqbit-to-librqbit smoke test; no daemon multi-file ingress yet.
use downloader_core::protocols::torrent::{gated_storage::GatedFilesystemFactory, multi_file_metadata,
    recovery_metadata, recovery_piece_verify, storage_gate::StorageGate};
use librqbit::{AddTorrent, AddTorrentOptions, ListenerMode, ListenerOptions, Session, SessionOptions, storage::StorageFactoryExt};
use std::{fs::{self, File}, os::unix::fs::{MetadataExt, PermissionsExt}, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gated_default_storage_downloads_then_quiesces_before_verification() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rget-gated-live-{}-{nonce}", std::process::id()));
    let seed_root = root.join("seed"); let seed_payload = seed_root.join("bundle");
    let download_root = root.join("download"); let payload = download_root.join("payload");
    fs::create_dir_all(seed_payload.join("dir")).unwrap(); fs::create_dir_all(&payload).unwrap();
    fs::set_permissions(&payload, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(seed_payload.join("dir/a"), b"abc").unwrap(); fs::write(seed_payload.join("empty"), b"").unwrap();
    let mut bytes = b"d4:infod5:filesld6:lengthi3e4:pathl3:dir1:aee".to_vec();
    bytes.extend_from_slice(b"d6:lengthi0e4:pathl5:emptyeee4:name6:bundle12:piece lengthi16384e6:pieces20:");
    bytes.extend_from_slice(&[0xa9,0x99,0x3e,0x36,0x47,0x06,0x81,0x6a,0xba,0x3e,0x25,0x71,0x78,0x50,0xc2,0x6c,0x9c,0xd0,0xd8,0x9d]);
    bytes.extend_from_slice(b"ee");
    let hash = librqbit::torrent_from_bytes(&bytes).unwrap().info_hash.as_string();
    let manifest = multi_file_metadata::preflight_local(&bytes).unwrap().unwrap();
    let info = recovery_metadata::from_metainfo(&bytes, &hash, &manifest).unwrap();
    let seed = Session::new_with_opts(seed_root.clone(), SessionOptions {
        dht: None, disable_trackers: true, disable_local_service_discovery: true,
        listen: Some(ListenerOptions { mode: ListenerMode::TcpOnly, listen_addr: ([127, 0, 0, 1], 0).into(), ..Default::default() }),
        ..Default::default()
    }).await.unwrap();
    let peer = seed.listen_addr().unwrap();
    let seed_handle = seed.add_torrent(AddTorrent::from_bytes(bytes.clone()), Some(AddTorrentOptions {
        overwrite: true, output_folder: Some(seed_payload.to_str().unwrap().into()), ..Default::default()
    })).await.unwrap().into_handle().unwrap();
    tokio::time::timeout(Duration::from_secs(15), seed_handle.wait_until_initialized()).await.unwrap().unwrap();
    assert!(seed_handle.stats().finished);
    let gate = StorageGate::new();
    let downloader = Session::new_with_opts(download_root, SessionOptions {
        dht: None, disable_trackers: true, disable_local_service_discovery: true, disable_upload: true,
        ..Default::default()
    }).await.unwrap();
    let handle = downloader.add_torrent(AddTorrent::from_bytes(bytes), Some(AddTorrentOptions {
        output_folder: Some(payload.to_str().unwrap().into()), initial_peers: Some(vec![peer]),
        storage_factory: Some(GatedFilesystemFactory { gate: gate.clone() }.boxed()), ..Default::default()
    })).await.unwrap().into_handle().unwrap();
    tokio::time::timeout(Duration::from_secs(30), handle.wait_until_completed()).await.unwrap().unwrap();
    downloader.stop().await;
    let closer = gate.clone(); tokio::task::spawn_blocking(move || closer.close_and_wait()).await.unwrap().unwrap();
    assert!(gate.enter().is_err());
    let dir = File::open(&payload).unwrap(); recovery_piece_verify::verify(&dir, dir.metadata().unwrap().dev(), &manifest, &info, &hash).unwrap();
    assert_eq!(fs::read(payload.join("dir/a")).unwrap(), b"abc");
    assert_eq!(fs::metadata(payload.join("empty")).unwrap().len(), 0);
    seed.stop().await; drop(handle); drop(downloader); drop(seed); fs::remove_dir_all(root).unwrap();
}
