//! 파일의 증가분을 읽고, 인코딩을 해석해 개행 단위의 텍스트를 반환한다.

use anyhow::{Result, bail};
use encoding_rs::Encoding;
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
};

/// 이번 읽기의 실제 줄 번호와 내용, 파일 교체·잘림 감지 여부.
pub struct Batch {
    pub lines: Vec<(u64, String)>,
    pub reset: bool,
}
// offset은 읽은 바이트 위치, line은 완료한 줄 수다. 미완성 줄은 pending에 남는다.
pub struct Tail {
    path: PathBuf,
    encoding: &'static Encoding,
    offset: u64,
    line: u64,
    pending: Vec<u8>,
    // 이전 오프셋 직전 최대 64바이트. 파일 크기가 같은 덮어쓰기도 비교할 수 있다.
    anchor: Vec<u8>,
    identity: u64,
    pub generation: u64,
    pub replacements: u64,
    pub oversized_lines: u64,
    discarding: bool,
}
// 경로에 새 파일이 들어왔는지 판단할 플랫폼별 단서. Windows는 생성 시각, Unix는 inode를 쓴다.
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
        // encoding_rs는 알려진 인코딩 테이블의 정적 참조를 돌려준다.
        let Some(encoding) = Encoding::for_label(encoding.as_bytes()) else {
            bail!("Unsupported encoding: {encoding}")
        };
        // 디코딩 전에 LF 바이트로 나누므로 바이트 수준에서 ASCII 호환인 인코딩만 받는다.
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
    // 청크 경계와 줄 경계는 다르다. LF까지 pending에 이어 붙여 멀티바이트 문자의 분할을 견딘다.
    fn consume(&mut self, bytes: &[u8]) -> Vec<(u64, String)> {
        let mut lines = vec![];
        // 최대 64KiB를 보관하고 초과분은 개행까지 버린다. 잘림은 한 줄당 한 번 센다.
        for &b in bytes {
            if b == b'\n' {
                self.line += 1;
                // Windows CRLF에서 CR을 떼어 Unix LF와 동일한 내용으로 만든다.
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                // 오류 수는 치환 문자 개수가 아니라 디코딩 오류가 있었던 줄 수다.
                let (value, _, errors) = self.encoding.decode(&self.pending);
                if errors {
                    self.replacements += 1;
                }
                // 디코더가 빌린 Cow를 돌려줄 수 있다. pending을 비우기 전에 소유 문자열로 바꾼다.
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
    /// 시작 시점의 파일 크기만큼 앞에서 읽어 실제 파일 줄 번호를 계산한다.
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
            // 마지막 keep개 완성 줄만 남겨 초기 읽기의 메모리 사용을 줄인다.
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
    // 읽기를 마친 위치 바로 앞의 내용을 다음 폴링에서 비교할 수 있게 저장한다.
    fn save_anchor(&mut self, file: &mut File) -> io::Result<()> {
        let len = self.offset.min(64) as usize;
        file.seek(SeekFrom::Start(self.offset - len as u64))?;
        self.anchor.resize(len, 0);
        file.read_exact(&mut self.anchor)
    }
    /// 매번 경로를 다시 열어 교체를 확인하고 마지막 위치부터 최대 256KiB를 읽는다.
    pub fn poll(&mut self) -> Result<Batch> {
        let mut file = File::open(&self.path)?;
        let id = identity(&file)?;
        let mut reset = id != self.identity || file.metadata()?.len() < self.offset;
        // 식별 정보가 같고 파일이 줄어들지 않았어도 기존 오프셋 직전의 바이트를 비교한다.
        if !reset && !self.anchor.is_empty() {
            file.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut check = vec![0; self.anchor.len()];
            reset = file.read_exact(&mut check).is_err() || check != self.anchor;
        }
        // 세대가 바뀌면 이전 파일의 미완성 바이트와 줄 번호를 폐기한다.
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
    // --dump에서만 마지막 개행을 가정한다. 라이브 모드는 실제 개행을 기다린다.
    pub fn finish(&mut self) -> Vec<(u64, String)> {
        if self.pending.is_empty() && !self.discarding {
            vec![]
        } else {
            self.consume(b"\n")
        }
    }
}
