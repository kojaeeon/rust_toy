use anyhow::{Result, bail};
use encoding_rs::Encoding;
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
};

pub struct Batch {
    pub lines: Vec<(u64, String)>,
    pub reset: bool,
}
pub struct Tail {
    path: PathBuf,
    encoding: &'static Encoding,
    offset: u64,
    line: u64,
    pending: Vec<u8>,
    anchor: Vec<u8>,
    identity: u64,
    pub generation: u64,
    pub replacements: u64,
    pub oversized_lines: u64,
    discarding: bool,
}
fn identity(file: &File) -> io::Result<u64> {
    let meta = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(meta.creation_time())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(meta.ino())
    }
    #[cfg(not(any(windows, unix)))]
    {
        Ok(0)
    }
}
impl Tail {
    pub fn new(path: PathBuf, encoding: &str) -> Result<Self> {
        let Some(encoding) = Encoding::for_label(encoding.as_bytes()) else {
            bail!("Unsupported encoding: {encoding}")
        };
        if !encoding.is_ascii_compatible() {
            bail!(
                "Use an ASCII-compatible encoding (UTF-8 or EUC-KR/CP949); UTF-16 is not supported"
            )
        }
        Ok(Self {
            path,
            encoding,
            offset: 0,
            line: 0,
            pending: vec![],
            anchor: vec![],
            identity: 0,
            generation: 1,
            replacements: 0,
            oversized_lines: 0,
            discarding: false,
        })
    }
    fn consume(&mut self, bytes: &[u8]) -> Vec<(u64, String)> {
        let mut lines = vec![];
        for &b in bytes {
            if b == b'\n' {
                self.line += 1;
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let (value, _, errors) = self.encoding.decode(&self.pending);
                if errors {
                    self.replacements += 1;
                }
                let mut value = value.into_owned();
                if self.discarding {
                    value.push_str(" [line truncated at 64 KiB]");
                }
                lines.push((self.line, value));
                self.pending.clear();
                self.discarding = false;
            } else if self.pending.len() < 65536 {
                self.pending.push(b);
            } else if !self.discarding {
                self.discarding = true;
                self.oversized_lines += 1;
            }
        }
        lines
    }
    pub fn initial(&mut self, keep: usize) -> Result<Batch> {
        let mut file = File::open(&self.path)?;
        self.identity = identity(&file)?;
        let mut remaining = file.metadata()?.len();
        let mut retained = VecDeque::new();
        let mut buf = vec![0; 65536];
        while remaining > 0 {
            let limit = remaining.min(buf.len() as u64) as usize;
            let count = file.read(&mut buf[..limit])?;
            if count == 0 {
                break;
            }
            remaining -= count as u64;
            self.offset += count as u64;
            for line in self.consume(&buf[..count]) {
                retained.push_back(line);
                if retained.len() > keep {
                    retained.pop_front();
                }
            }
        }
        self.save_anchor(&mut file)?;
        Ok(Batch {
            lines: retained.into_iter().collect(),
            reset: false,
        })
    }
    fn save_anchor(&mut self, file: &mut File) -> io::Result<()> {
        let len = self.offset.min(64) as usize;
        file.seek(SeekFrom::Start(self.offset - len as u64))?;
        self.anchor.resize(len, 0);
        file.read_exact(&mut self.anchor)
    }
    pub fn poll(&mut self) -> Result<Batch> {
        let mut file = File::open(&self.path)?;
        let id = identity(&file)?;
        let mut reset = id != self.identity || file.metadata()?.len() < self.offset;
        if !reset && !self.anchor.is_empty() {
            file.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut check = vec![0; self.anchor.len()];
            reset = file.read_exact(&mut check).is_err() || check != self.anchor;
        }
        if reset {
            self.offset = 0;
            self.line = 0;
            self.pending.clear();
            self.discarding = false;
            self.generation += 1;
            self.identity = id;
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = vec![0; 256 * 1024];
        let count = file.read(&mut bytes)?;
        self.offset += count as u64;
        let lines = self.consume(&bytes[..count]);
        self.save_anchor(&mut file)?;
        Ok(Batch { lines, reset })
    }
    // Only for a finite --dump snapshot: live mode keeps incomplete bytes until newline arrives.
    pub fn finish(&mut self) -> Vec<(u64, String)> {
        if self.pending.is_empty() && !self.discarding {
            vec![]
        } else {
            self.consume(b"\n")
        }
    }
}
