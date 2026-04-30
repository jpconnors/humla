// GGUF header sniffer. Reads enough of a .gguf file to identify architecture
// and quantization without loading the model weights. Used by:
// 1. local_llm_download to validate a freshly-downloaded file is what we expect
// 2. llm_discovery to filter scanned-from-disk models for compatibility
//
// Format reference: https://github.com/ggml-org/ggml/blob/master/docs/gguf.md
// We support GGUF v2 and v3 (both currently produced by llama.cpp tooling).

use anyhow::{anyhow, bail, Result};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct GgufInfo {
    pub architecture: String,
    pub quantization: String,
    pub parameter_count: Option<u64>,
}

pub fn sniff(path: &Path) -> Result<GgufInfo> {
    let f = File::open(path)?;
    let mut r = BufReader::new(f);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        bail!("not a GGUF file");
    }
    let version = read_u32(&mut r)?;
    if !(2..=3).contains(&version) {
        bail!("unsupported GGUF version {}", version);
    }
    let _tensor_count = read_u64(&mut r)?;
    let kv_count = read_u64(&mut r)?;

    let mut architecture: Option<String> = None;
    let mut file_type: Option<u32> = None;
    let mut parameter_count: Option<u64> = None;

    for _ in 0..kv_count {
        let key = read_string(&mut r)?;
        let value_type = read_u32(&mut r)?;
        match (key.as_str(), value_type) {
            ("general.architecture", 8) => {
                architecture = Some(read_string(&mut r)?);
            }
            ("general.file_type", 4) => {
                file_type = Some(read_u32(&mut r)?);
            }
            ("general.parameter_count", 11) => {
                parameter_count = Some(read_u64(&mut r)?);
            }
            _ => skip_value(&mut r, value_type)?,
        }
        if architecture.is_some() && file_type.is_some() {
            break;
        }
    }

    Ok(GgufInfo {
        architecture: architecture.ok_or_else(|| anyhow!("missing general.architecture"))?,
        quantization: file_type.map(quant_label).unwrap_or_else(|| "unknown".into()),
        parameter_count,
    })
}

fn quant_label(file_type: u32) -> String {
    // Subset of llama.cpp's enum llama_ftype — only the values we actually surface.
    match file_type {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        _ => "other",
    }
    .into()
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_string<R: Read>(r: &mut R) -> Result<String> {
    // Sanity ceiling — Gemma's chat template metadata is ~16 KB, tokenizer
    // pre-tokenizer regexes can be larger. 1 MB is comfortable headroom and
    // still rejects an obvious garbage file where length bytes happen to
    // decode to gigabyte-sized values.
    let len = read_u64(r)? as usize;
    if len > 1_048_576 {
        bail!("gguf string too long: {len}");
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(String::from_utf8(buf)?)
}

fn skip_value<R: Read + Seek>(r: &mut R, value_type: u32) -> Result<()> {
    match value_type {
        0 | 1 | 7 => {
            r.seek(SeekFrom::Current(1))?;
        }
        2 | 3 => {
            r.seek(SeekFrom::Current(2))?;
        }
        4 | 5 | 6 => {
            r.seek(SeekFrom::Current(4))?;
        }
        10 | 11 | 12 => {
            r.seek(SeekFrom::Current(8))?;
        }
        8 => {
            let _ = read_string(r)?;
        }
        9 => {
            let array_value_type = read_u32(r)?;
            let array_len = read_u64(r)?;
            for _ in 0..array_len {
                skip_value(r, array_value_type)?;
            }
        }
        other => bail!("unknown gguf value type {other}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_minimal_gguf(arch: &str, file_type: u32) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"GGUF").unwrap();
        f.write_all(&3u32.to_le_bytes()).unwrap();
        f.write_all(&0u64.to_le_bytes()).unwrap();
        f.write_all(&2u64.to_le_bytes()).unwrap();
        let key = b"general.architecture";
        f.write_all(&(key.len() as u64).to_le_bytes()).unwrap();
        f.write_all(key).unwrap();
        f.write_all(&8u32.to_le_bytes()).unwrap();
        f.write_all(&(arch.len() as u64).to_le_bytes()).unwrap();
        f.write_all(arch.as_bytes()).unwrap();
        let key = b"general.file_type";
        f.write_all(&(key.len() as u64).to_le_bytes()).unwrap();
        f.write_all(key).unwrap();
        f.write_all(&4u32.to_le_bytes()).unwrap();
        f.write_all(&file_type.to_le_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn parses_gemma_q4_k_m() {
        let f = write_minimal_gguf("gemma3", 15);
        let info = sniff(f.path()).unwrap();
        assert_eq!(info.architecture, "gemma3");
        assert_eq!(info.quantization, "Q4_K_M");
    }

    #[test]
    fn parses_qwen_q8_0() {
        let f = write_minimal_gguf("qwen3", 7);
        let info = sniff(f.path()).unwrap();
        assert_eq!(info.architecture, "qwen3");
        assert_eq!(info.quantization, "Q8_0");
    }

    #[test]
    fn rejects_non_gguf() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"NOT_GGUF_AT_ALL_NO_WAY").unwrap();
        f.flush().unwrap();
        assert!(sniff(f.path()).is_err());
    }
}
