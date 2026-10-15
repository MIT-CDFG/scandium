// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §3.4 (scratch disk format — normative extent layout), §3.8
//! (codecs), §7.8 (the autosave journal fsyncs scratch extents BEFORE the
//! TILEREF referencing them; transient spill is never fsync'd).
//!
//! [NORMATIVE, §3.4] The scratch file is a sequence of fixed **4 MiB
//! extents**: one 4 KiB header block (magic `APSCRT01`, version 1, owning
//! document id, allocation bitmap) plus 1023 payload blocks of 4 KiB. Blobs
//! occupy contiguous block runs, first-fit within an extent, tracked by
//! per-extent in-memory free lists — the on-disk bitmap is written lazily
//! for inspection only; the AUTHORITATIVE state is in RAM because spill
//! data is transient (never valid across a crash — recovery uses the
//! journal; stale scratch files are deleted at launch). Compaction: none in
//! v1 [NORMATIVE]; freed runs are reused and a wholly-free tail extent MAY
//! be truncated.
//!
//! The spec's `ScratchRef` shape is kept verbatim (extent, first_block,
//! blocks, raw_len, codec). The compressed byte length is NOT part of the
//! on-disk shape; callers keep it beside the ref in RAM (the
//! authoritative-in-RAM rule above) and pass it back to [`ScratchDisk::read`].

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use uuid::Uuid;

use crate::tile::compress::TileCodec;

pub(crate) const EXTENT_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const BLOCK_BYTES: usize = 4096;
pub(crate) const BLOCKS_PER_EXTENT: usize = 1023;
/// Scratch-extent magic. IMPLEMENTATION-OWNED (not a §7 published format):
/// scratch files are transient — deleted at launch, never valid across a
/// crash — so the tag is freely renameable; it now follows the D-43
/// placeholder identifier family (O-WATCH MINOR-11).
const MAGIC: &[u8; 8] = b"APSCRT01";

/// §3.4 blob reference (spec-verbatim shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScratchRef {
    pub extent: u32,
    pub first_block: u16,
    pub blocks: u16,
    pub raw_len: u32,
    pub codec: TileCodec,
}

struct ExtentState {
    /// bit i set = payload block i occupied (in-RAM authoritative).
    bitmap: [u64; 16],
}

impl ExtentState {
    fn new() -> ExtentState {
        ExtentState { bitmap: [0; 16] }
    }

    fn occupied(&self, i: usize) -> bool {
        self.bitmap[i / 64] & (1 << (i % 64)) != 0
    }

    fn set(&mut self, i: usize, on: bool) {
        if on {
            self.bitmap[i / 64] |= 1 << (i % 64);
        } else {
            self.bitmap[i / 64] &= !(1 << (i % 64));
        }
    }

    /// First-fit contiguous run of `n` free blocks.
    fn find_run(&self, n: usize) -> Option<usize> {
        let mut run = 0usize;
        for i in 0..BLOCKS_PER_EXTENT {
            if self.occupied(i) {
                run = 0;
            } else {
                run += 1;
                if run == n {
                    return Some(i + 1 - n);
                }
            }
        }
        None
    }

    fn is_empty(&self) -> bool {
        self.bitmap.iter().all(|w| *w == 0)
    }
}

/// The per-engine scratch file (§3.4). Created lazily in the configured
/// directory; the file is removed on drop (transient by contract).
pub struct ScratchDisk {
    path: PathBuf,
    file: File,
    doc_id: Uuid,
    extents: Vec<ExtentState>,
}

impl ScratchDisk {
    /// Create a fresh scratch file for one document in `dir`. The file name
    /// carries the pid + doc id so concurrent engines never collide; stale
    /// files from crashed sessions are simply never reopened (spec: deleted
    /// at launch after the orphan-journal scan — the CLI's launch path owns
    /// that sweep).
    pub fn create(dir: &std::path::Path, doc_id: Uuid) -> std::io::Result<ScratchDisk> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            "appname-scratch-{}-{doc_id}.bgscratch",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        Ok(ScratchDisk {
            path,
            file,
            doc_id,
            extents: Vec::new(),
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn grow_extent(&mut self) -> std::io::Result<usize> {
        let idx = self.extents.len();
        // Header block: magic | version u16 LE | reserved(6) | doc id (16) |
        // bitmap (128, lazily maintained — zero at creation) | pad to 4096.
        let mut header = vec![0u8; BLOCK_BYTES];
        header[0..8].copy_from_slice(MAGIC);
        header[8..10].copy_from_slice(&1u16.to_le_bytes());
        header[16..32].copy_from_slice(self.doc_id.as_bytes());
        self.file.seek(SeekFrom::Start(idx as u64 * EXTENT_BYTES))?;
        self.file.write_all(&header)?;
        // Reserve the extent's full span so payload writes never race the
        // file length.
        self.file.set_len((idx as u64 + 1) * EXTENT_BYTES)?;
        self.extents.push(ExtentState::new());
        Ok(idx)
    }

    fn block_offset(extent: usize, block: usize) -> u64 {
        extent as u64 * EXTENT_BYTES + (block as u64 + 1) * BLOCK_BYTES as u64
    }

    /// Store one compressed blob; returns its §3.4 reference. `raw_len` is
    /// the uncompressed payload length the caller will revive to.
    pub fn alloc(
        &mut self,
        bytes: &[u8],
        raw_len: u32,
        codec: TileCodec,
    ) -> std::io::Result<ScratchRef> {
        let blocks = bytes.len().div_ceil(BLOCK_BYTES).max(1);
        assert!(
            blocks <= BLOCKS_PER_EXTENT,
            "blob exceeds one extent (§3.2 class sizes)"
        );
        let (extent, first) = match self
            .extents
            .iter()
            .enumerate()
            .find_map(|(e, st)| st.find_run(blocks).map(|b| (e, b)))
        {
            Some(hit) => hit,
            None => {
                let e = self.grow_extent()?;
                (e, 0)
            }
        };
        for b in first..first + blocks {
            self.extents[extent].set(b, true);
        }
        self.file
            .seek(SeekFrom::Start(Self::block_offset(extent, first)))?;
        self.file.write_all(bytes)?;
        // Transient spill is never fsync'd (§3.4); the §7.8 journal path
        // calls [`ScratchDisk::sync`] explicitly before appending TILEREFs.
        Ok(ScratchRef {
            extent: extent as u32,
            first_block: first as u16,
            blocks: blocks as u16,
            raw_len,
            codec,
        })
    }

    /// Read a blob back. `comp_len` is the exact compressed byte length the
    /// caller recorded beside the ref (in-RAM authoritative sidecar).
    pub fn read(&mut self, r: &ScratchRef, comp_len: usize) -> std::io::Result<Vec<u8>> {
        assert!(comp_len <= r.blocks as usize * BLOCK_BYTES);
        let mut out = vec![0u8; comp_len];
        self.file.seek(SeekFrom::Start(Self::block_offset(
            r.extent as usize,
            r.first_block as usize,
        )))?;
        self.file.read_exact(&mut out)?;
        Ok(out)
    }

    /// Return a blob's blocks to the free pool; a wholly-free tail extent
    /// is truncated (§3.4 MAY).
    pub fn free(&mut self, r: &ScratchRef) {
        let st = &mut self.extents[r.extent as usize];
        for b in r.first_block as usize..(r.first_block + r.blocks) as usize {
            st.set(b, false);
        }
        while let Some(last) = self.extents.last() {
            if !last.is_empty() {
                break;
            }
            self.extents.pop();
            let _ = self.file.set_len(self.extents.len() as u64 * EXTENT_BYTES);
        }
    }

    /// Durability door for the §7.8 journal path (extents are fsync'd
    /// BEFORE the TILEREF referencing them is appended).
    pub fn sync(&self) -> std::io::Result<()> {
        self.file.sync_all()
    }

    pub fn extent_count(&self) -> usize {
        self.extents.len()
    }

    pub fn disk_bytes(&self) -> u64 {
        self.extents.len() as u64 * EXTENT_BYTES
    }
}

impl Drop for ScratchDisk {
    fn drop(&mut self) {
        // Transient by contract: never valid across a crash; remove on
        // clean teardown.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk() -> ScratchDisk {
        let dir = std::env::temp_dir().join(format!("appname-scratch-test-{}", Uuid::new_v4()));
        ScratchDisk::create(&dir, Uuid::new_v4()).unwrap()
    }

    #[test]
    fn alloc_read_free_round_trips() {
        let mut d = disk();
        let blob: Vec<u8> = (0..10_000u32).map(|i| (i % 255) as u8).collect();
        let r = d.alloc(&blob, 20_000, TileCodec::Zstd).unwrap();
        assert_eq!(r.blocks as usize, blob.len().div_ceil(BLOCK_BYTES));
        assert_eq!(d.read(&r, blob.len()).unwrap(), blob);
        assert_eq!(d.extent_count(), 1);
        d.free(&r);
        assert_eq!(d.extent_count(), 0, "wholly-free tail extent truncates");
    }

    #[test]
    fn first_fit_reuses_freed_runs() {
        let mut d = disk();
        let a = d.alloc(&[1u8; 9000], 9000, TileCodec::None).unwrap();
        let b = d.alloc(&[2u8; 5000], 5000, TileCodec::None).unwrap();
        assert_eq!((a.extent, a.first_block), (0, 0));
        assert_eq!(b.first_block, a.first_block + a.blocks);
        d.free(&a);
        // The freed head run is reused first-fit.
        let c = d.alloc(&[3u8; 4000], 4000, TileCodec::None).unwrap();
        assert_eq!((c.extent, c.first_block), (0, 0));
        assert_eq!(d.read(&b, 5000).unwrap(), vec![2u8; 5000]);
        assert_eq!(d.read(&c, 4000).unwrap(), vec![3u8; 4000]);
    }

    #[test]
    fn grows_extents_and_header_is_normative() {
        let mut d = disk();
        // Fill more than one extent (1023 blocks) with 8-block blobs.
        let blob = vec![7u8; 8 * BLOCK_BYTES];
        let n = BLOCKS_PER_EXTENT / 8 + 2;
        let refs: Vec<ScratchRef> = (0..n)
            .map(|_| d.alloc(&blob, blob.len() as u32, TileCodec::None).unwrap())
            .collect();
        assert!(d.extent_count() >= 2, "second extent grew");
        assert!(refs.iter().any(|r| r.extent == 1));
        // Header block of extent 0: magic + version + doc id offset layout.
        let mut f = std::fs::File::open(d.path()).unwrap();
        let mut header = [0u8; 32];
        f.read_exact(&mut header).unwrap();
        assert_eq!(&header[0..8], b"APSCRT01");
        assert_eq!(u16::from_le_bytes([header[8], header[9]]), 1);
        for r in &refs {
            assert_eq!(d.read(r, blob.len()).unwrap(), blob);
        }
    }

    #[test]
    fn file_removed_on_drop() {
        let d = disk();
        let path = d.path().to_path_buf();
        drop(d);
        assert!(!path.exists());
    }
}
