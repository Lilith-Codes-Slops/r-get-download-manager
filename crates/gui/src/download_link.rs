//! Validates links and chooses the same display names as the reference.
use url::Url;

pub const INVALID_LINK: &str = "Enter a valid http:// or https:// link, or a magnet: link.";

pub fn parse(input: &str) -> Result<(String, String), &'static str> {
    let source = input.trim();
    if source
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("magnet:"))
    {
        // Mirror /[?&]dn=([^&]+)/ from the reference, including links
        // that use an ampersand directly after the magnet scheme.
        let name = source
            .char_indices()
            .filter(|(_, ch)| *ch == '?' || *ch == '&')
            .find_map(|(offset, _)| {
                let rest = &source[offset + 1..];
                let value = rest.strip_prefix("dn=")?.split('&').next()?;
                (!value.is_empty()).then(|| decode_or_original(&value.replace('+', " ")))
            })
            .unwrap_or_else(|| "magnet-download".to_owned());
        return Ok((source.to_owned(), name));
    }
    let url = Url::parse(source).map_err(|_| INVALID_LINK)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(INVALID_LINK);
    }
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .unwrap_or("download");
    Ok((source.to_owned(), decode_or_original(segment)))
}

// decodeURIComponent rejects malformed escapes and invalid UTF-8; keep the
// original segment in those cases, exactly like the reference's safeDecode.
fn decode_or_original(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return text.to_owned();
            }
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => decoded.push(hi * 16 + lo),
                _ => return text.to_owned(),
            }
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_choose_names_and_keep_original_source() {
        assert_eq!(
            parse(" https://example.com/folder/a%20b.iso?q=1 ").unwrap(),
            (
                "https://example.com/folder/a%20b.iso?q=1".into(),
                "a b.iso".into()
            )
        );
        assert_eq!(parse("https://example.com/").unwrap().1, "download");
        assert_eq!(
            parse("magnet:?xt=urn:btih:abc&dn=My+file%20%E2%9C%93")
                .unwrap()
                .1,
            "My file ✓"
        );
        assert_eq!(
            parse("MAGNET:?xt=urn:btih:abc").unwrap().1,
            "magnet-download"
        );
        assert_eq!(parse("magnet:?dn=bad%FF").unwrap().1, "bad%FF");
        assert_eq!(parse("magnet:abc&dn=example").unwrap().1, "example");
    }
    #[test]
    fn rejects_invalid_and_non_http_sources() {
        for value in [
            "",
            "not a url",
            "ftp://example.com/file",
            "http://",
            "https://bad host/a",
            "https://example.com:99999/",
        ] {
            assert!(parse(value).is_err(), "{value}");
        }
    }
}
