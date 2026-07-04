use std::fs::{self, File};
use std::io::{Seek, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use memmap2::Mmap;

/// On-disk line corpus: original text and lowercase copy, memory-mapped at search time.
#[derive(Debug)]
pub struct LineCorpus {
    path: PathBuf,
    mmap: Option<Mmap>,
    text_region_len: usize,
}

#[derive(Debug, Clone)]
pub struct LineRef {
    pub path: String,
    pub line_number: u64,
    pub text_off: u32,
    pub text_len: u32,
    pub lower_off: u32,
}

impl LineCorpus {
    pub fn open(path: PathBuf) -> Result<Self> {
        let mmap = if path.is_file() {
            let file = File::open(&path).context("open corpus")?;
            Some(unsafe { Mmap::map(&file).context("mmap corpus")? })
        } else {
            None
        };
        let text_region_len = mmap
            .as_ref()
            .and_then(|m| m.iter().position(|&b| b == 0))
            .unwrap_or(0);
        Ok(Self {
            path,
            mmap,
            text_region_len,
        })
    }

    pub fn reset(&mut self) -> Result<File> {
        self.mmap = None;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = File::create(&self.path).context("create corpus")?;
        Ok(file)
    }

    pub fn append_line(
        writer: &mut File,
        lower_writer: &mut Vec<u8>,
        path: &str,
        line_number: u64,
        text: &str,
    ) -> Result<LineRef> {
        let text_off = writer.stream_position()? as u32;
        let text_bytes = text.as_bytes();
        writer.write_all(text_bytes)?;
        let text_len = text_bytes.len() as u32;

        let lower_off = lower_writer.len() as u32;
        lower_writer.extend(text.to_lowercase().bytes());

        Ok(LineRef {
            path: path.to_string(),
            line_number,
            text_off,
            text_len,
            lower_off,
        })
    }

    pub fn finalize(mut self, mut writer: File, lower_region: &[u8]) -> Result<Self> {
        writer.write_all(&[0])?;
        writer.write_all(lower_region)?;
        writer.sync_all()?;
        drop(writer);
        self.mmap = None;
        let file = File::open(&self.path)?;
        let mmap = unsafe { Mmap::map(&file).context("mmap finalized corpus")? };
        self.text_region_len = mmap.iter().position(|&b| b == 0).unwrap_or(mmap.len());
        self.mmap = Some(mmap);
        Ok(self)
    }

    pub fn text_at(&self, line: &LineRef) -> &str {
        let Some(mmap) = &self.mmap else {
            return "";
        };
        let start = line.text_off as usize;
        let end = start + line.text_len as usize;
        std::str::from_utf8(&mmap[start..end]).unwrap_or("")
    }

    pub fn lower_at(&self, line: &LineRef) -> &str {
        let Some(mmap) = &self.mmap else {
            return "";
        };
        let base = self.text_region_len + 1;
        let start = base + line.lower_off as usize;
        let end = start + line.text_len as usize;
        if end > mmap.len() {
            return "";
        }
        std::str::from_utf8(&mmap[start..end]).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_line_text() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("corpus.bin");
        let mut corpus = LineCorpus::open(path.clone()).unwrap();
        let mut writer = corpus.reset().unwrap();
        let mut lower = Vec::new();
        let line = LineCorpus::append_line(
            &mut writer,
            &mut lower,
            "a.rs",
            1,
            "Hello Thunder",
        )
        .unwrap();
        let corpus = corpus.finalize(writer, &lower).unwrap();
        assert_eq!(corpus.text_at(&line), "Hello Thunder");
        assert_eq!(corpus.lower_at(&line), "hello thunder");
    }
}
