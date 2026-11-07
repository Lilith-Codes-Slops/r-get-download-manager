use blake3::Hasher;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct Blake3Verifier;

impl Blake3Verifier {
    pub fn verify(path: &Path, expected: &[u8]) -> Result<bool, std::io::Error> {
        let mut file = File::open(path)?;
        let mut hasher = Hasher::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().as_bytes() == expected)
    }
}
