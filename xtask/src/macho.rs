//! Lowering the minimum macOS version of a linked, ad-hoc signed Mach-O.
//!
//! zig links darwin targets with its own default deployment target (macOS
//! 15 with zig 0.17) and cargo-zigbuild has no way to pass another, so the
//! binaries would refuse to start on older macOS although nothing in them
//! needs it. This rewrites `minos` in LC_BUILD_VERSION and re-hashes the
//! code page that holds it, which keeps the linker's ad-hoc signature valid
//! (an ad-hoc CodeDirectory is not itself signed; the kernel only checks that
//! each page matches its hash).

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_BUILD_VERSION: u32 = 0x32;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;
const CS_HASHTYPE_SHA256: u8 = 2;

fn le32(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at + 4).context("truncated Mach-O")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at + 4).context("truncated code signature")?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// `12.0` → 0x000c0000 (the X.Y.Z nibble encoding of Mach-O versions).
pub fn encode_version(v: &str) -> Result<u32> {
    let mut parts = v.split('.').map(|p| p.parse::<u32>());
    let major = parts.next().context("empty version")??;
    let minor = parts.next().transpose()?.unwrap_or(0);
    let patch = parts.next().transpose()?.unwrap_or(0);
    if major > 0xffff || minor > 0xff || patch > 0xff {
        bail!("version {v} does not fit");
    }
    Ok((major << 16) | (minor << 8) | patch)
}

struct CodeDirectory {
    /// Absolute offset of the first code slot hash.
    hashes: usize,
    slots: usize,
    page_size: usize,
    code_limit: usize,
}

fn code_directory(bin: &[u8], sig_off: usize) -> Result<CodeDirectory> {
    if be32(bin, sig_off)? != CSMAGIC_EMBEDDED_SIGNATURE {
        bail!("no embedded signature");
    }
    let count = be32(bin, sig_off + 8)? as usize;
    for i in 0..count {
        let typ = be32(bin, sig_off + 12 + i * 8)?;
        let off = be32(bin, sig_off + 16 + i * 8)? as usize;
        if typ != 0 {
            continue;
        }
        let cd = sig_off + off;
        if be32(bin, cd)? != CSMAGIC_CODEDIRECTORY {
            bail!("slot 0 is not a CodeDirectory");
        }
        let hash_offset = be32(bin, cd + 16)? as usize;
        let slots = be32(bin, cd + 28)? as usize;
        let code_limit = be32(bin, cd + 32)? as usize;
        let hash_size = *bin.get(cd + 36).context("truncated CodeDirectory")?;
        let hash_type = *bin.get(cd + 37).context("truncated CodeDirectory")?;
        let page_log2 = *bin.get(cd + 39).context("truncated CodeDirectory")?;
        if hash_type != CS_HASHTYPE_SHA256 || hash_size != 32 {
            bail!("unexpected hash type {hash_type} (size {hash_size})");
        }
        if !(9..=16).contains(&page_log2) {
            bail!("unexpected page size 2^{page_log2}");
        }
        return Ok(CodeDirectory {
            hashes: cd + hash_offset,
            slots,
            page_size: 1 << page_log2,
            code_limit,
        });
    }
    bail!("no CodeDirectory in the signature")
}

fn page_hash(bin: &[u8], cd: &CodeDirectory, page: usize) -> [u8; 32] {
    let start = page * cd.page_size;
    let end = ((page + 1) * cd.page_size).min(cd.code_limit);
    Sha256::digest(&bin[start..end]).into()
}

fn slot(bin: &[u8], cd: &CodeDirectory, page: usize) -> Result<[u8; 32]> {
    let at = cd.hashes + page * 32;
    let s = bin.get(at..at + 32).context("truncated hash slot")?;
    let mut out = [0u8; 32];
    out.copy_from_slice(s);
    Ok(out)
}

/// Every page matches its hash (true for unsigned binaries).
pub fn signature_is_consistent(bin: &[u8]) -> Result<bool> {
    let (_, Some(sig)) = locate(bin)? else {
        return Ok(true);
    };
    let cd = code_directory(bin, sig)?;
    for page in 0..cd.slots {
        if page_hash(bin, &cd, page) != slot(bin, &cd, page)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Offsets of LC_BUILD_VERSION's minos field and of the code signature (if
/// any: zig signs arm64 binaries, which macOS requires, but not x86_64 ones).
fn locate(bin: &[u8]) -> Result<(usize, Option<usize>)> {
    if le32(bin, 0)? != MH_MAGIC_64 {
        bail!("not a 64-bit Mach-O (fat binaries are not supported)");
    }
    let ncmds = le32(bin, 16)? as usize;
    let mut off = 32;
    let (mut minos, mut sig) = (None, None);
    for _ in 0..ncmds {
        let cmd = le32(bin, off)?;
        let size = le32(bin, off + 4)? as usize;
        match cmd {
            LC_BUILD_VERSION => minos = Some(off + 12),
            LC_CODE_SIGNATURE => sig = Some(le32(bin, off + 8)? as usize),
            _ => {}
        }
        if size == 0 {
            bail!("malformed load command");
        }
        off += size;
    }
    Ok((minos.context("no LC_BUILD_VERSION")?, sig))
}

/// Sets the minimum macOS version; returns the previous one (encoded).
pub fn set_min_macos(bin: &mut [u8], version: &str) -> Result<u32> {
    let want = encode_version(version)?;
    let (minos_at, sig) = locate(bin)?;
    let cputype = le32(bin, 4)?;
    let before = le32(bin, minos_at)?;
    let Some(sig) = sig else {
        if cputype == 0x0100_000c {
            bail!("an unsigned arm64 binary would not run on macOS");
        }
        bin[minos_at..minos_at + 4].copy_from_slice(&want.to_le_bytes());
        return Ok(before);
    };
    let cd = code_directory(bin, sig)?;
    if !signature_is_consistent(bin)? {
        bail!("the signature does not match the binary before patching");
    }
    bin[minos_at..minos_at + 4].copy_from_slice(&want.to_le_bytes());
    let page = minos_at / cd.page_size;
    let hash = page_hash(bin, &cd, page);
    let at = cd.hashes + page * 32;
    bin[at..at + 32].copy_from_slice(&hash);
    if !signature_is_consistent(bin)? {
        bail!("the signature does not match the binary after patching");
    }
    Ok(before)
}

/// "15.0" from an encoded version.
pub fn decode_version(v: u32) -> String {
    format!("{}.{}", v >> 16, (v >> 8) & 0xff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(encode_version("12.0").unwrap(), 0x000c_0000);
        assert_eq!(encode_version("10.12").unwrap(), 0x000a_0c00);
        assert_eq!(decode_version(0x000f_0000), "15.0");
        assert!(encode_version("x").is_err());
    }

    /// A minimal signed Mach-O: header, LC_BUILD_VERSION, LC_CODE_SIGNATURE,
    /// two pages of "code", and an ad-hoc signature over them.
    fn fixture() -> Vec<u8> {
        let page = 4096usize;
        let code_limit = page + 100;
        let mut bin = vec![0u8; code_limit];
        bin[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        bin[16..20].copy_from_slice(&2u32.to_le_bytes()); // ncmds
        // LC_BUILD_VERSION at 32
        bin[32..36].copy_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        bin[36..40].copy_from_slice(&24u32.to_le_bytes());
        bin[40..44].copy_from_slice(&1u32.to_le_bytes()); // platform macOS
        bin[44..48].copy_from_slice(&0x000f_0000u32.to_le_bytes()); // minos 15.0
        // LC_CODE_SIGNATURE at 56
        bin[56..60].copy_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
        bin[60..64].copy_from_slice(&16u32.to_le_bytes());
        bin[64..68].copy_from_slice(&(code_limit as u32).to_le_bytes());
        for (i, b) in bin.iter_mut().enumerate().skip(200) {
            *b = (i % 251) as u8;
        }
        // SuperBlob + CodeDirectory (big-endian)
        let slots = 2usize;
        let cd_header = 44usize;
        let cd_len = cd_header + slots * 32;
        let mut sig = Vec::new();
        sig.extend(CSMAGIC_EMBEDDED_SIGNATURE.to_be_bytes());
        sig.extend(((20 + cd_len) as u32).to_be_bytes());
        sig.extend(1u32.to_be_bytes());
        sig.extend(0u32.to_be_bytes());
        sig.extend(20u32.to_be_bytes());
        let mut cd = vec![0u8; cd_header];
        cd[0..4].copy_from_slice(&CSMAGIC_CODEDIRECTORY.to_be_bytes());
        cd[4..8].copy_from_slice(&(cd_len as u32).to_be_bytes());
        cd[16..20].copy_from_slice(&(cd_header as u32).to_be_bytes());
        cd[28..32].copy_from_slice(&(slots as u32).to_be_bytes());
        cd[32..36].copy_from_slice(&(code_limit as u32).to_be_bytes());
        cd[36] = 32;
        cd[37] = CS_HASHTYPE_SHA256;
        cd[39] = 12;
        for p in 0..slots {
            let end = ((p + 1) * page).min(code_limit);
            cd.extend(Sha256::digest(&bin[p * page..end]));
        }
        sig.extend(cd);
        bin.extend(sig);
        bin
    }

    #[test]
    fn lowers_minos_and_keeps_the_signature() {
        let mut bin = fixture();
        assert!(signature_is_consistent(&bin).unwrap());
        let before = set_min_macos(&mut bin, "12.0").unwrap();
        assert_eq!(decode_version(before), "15.0");
        assert_eq!(le32(&bin, 44).unwrap(), 0x000c_0000);
        assert!(signature_is_consistent(&bin).unwrap());
    }

    /// `MACHO_SAMPLE=<a zig-linked darwin binary> cargo test -p xtask macho`
    #[test]
    fn real_binary_when_given() {
        let Ok(path) = std::env::var("MACHO_SAMPLE") else {
            return;
        };
        let mut bin = std::fs::read(path).unwrap();
        assert!(signature_is_consistent(&bin).unwrap());
        set_min_macos(&mut bin, "12.0").unwrap();
        assert!(signature_is_consistent(&bin).unwrap());
        let (minos, _) = locate(&bin).unwrap();
        assert_eq!(le32(&bin, minos).unwrap(), 0x000c_0000);
    }

    #[test]
    fn refuses_a_broken_signature() {
        let mut bin = fixture();
        bin[300] ^= 0xff;
        assert!(set_min_macos(&mut bin, "12.0").is_err());
    }
}
