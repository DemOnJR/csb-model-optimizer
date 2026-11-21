//! `.gz` copies for fast-download hosts (clients that support it ask for `file.gz` first).

use std::io::Write;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gzip {
    Off,
    /// zlib level 9: fast, the usual choice.
    Best,
    /// Zopfli: about 4-8% smaller, many times slower. The output is plain gzip.
    Zopfli,
}

/// gzip bytes with no file name and mtime 0, so the same input always gives the same output.
pub fn gzip(data: &[u8], mode: Gzip) -> Vec<u8> {
    match mode {
        Gzip::Off => Vec::new(),
        Gzip::Best => {
            let mut e = flate2::GzBuilder::new().mtime(0).write(Vec::new(), flate2::Compression::best());
            e.write_all(data).expect("write to memory");
            e.finish().expect("write to memory")
        }
        Gzip::Zopfli => {
            let mut out = Vec::new();
            let opts = zopfli::Options::default();
            zopfli::compress(opts, zopfli::Format::Gzip, data, &mut out).expect("write to memory");
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn round_trip() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        for m in [Gzip::Best, Gzip::Zopfli] {
            let z = gzip(&data, m);
            assert!(z.len() < data.len() / 4);
            let mut back = Vec::new();
            flate2::read::GzDecoder::new(&z[..]).read_to_end(&mut back).unwrap();
            assert_eq!(back, data);
            assert_eq!(z, gzip(&data, m), "deterministic");
        }
    }
}
