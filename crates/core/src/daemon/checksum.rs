//! Explicit verification of an already completed download.
use std::path::PathBuf;

pub(super) async fn verify(
    path: PathBuf,
    algorithm: String,
    expected: String,
) -> Result<bool, String> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("expected hash must be 64 hexadecimal characters".into());
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&expected[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    tokio::task::spawn_blocking(move || match algorithm.to_ascii_lowercase().as_str() {
        "sha256" | "sha-256" => {
            crate::integrity::sha256_verifier::Sha256Verifier::verify(&path, &bytes)
                .map_err(|e| e.to_string())
        }
        "blake3" => crate::integrity::blake3_verifier::Blake3Verifier::verify(&path, &bytes)
            .map_err(|e| e.to_string()),
        _ => Err("unsupported checksum algorithm; use sha256 or blake3".into()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validates_sha256_and_rejects_invalid_hash() {
        let path = std::env::temp_dir().join(format!("rget-checksum-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(path.clone(), "sha256".into(), hash.into())
            .await
            .unwrap());
        assert!(!verify(path.clone(), "sha256".into(), "00".repeat(32))
            .await
            .unwrap());
        assert!(verify(path.clone(), "sha256".into(), "invalid".into())
            .await
            .is_err());
        std::fs::remove_file(path).unwrap();
    }
}
