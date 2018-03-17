use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Seek};
use std::num::{ParseFloatError, ParseIntError};
use std::time::Duration;
use zip::read::ZipArchive;
use zip::result::ZipError;

pub struct CaseVec<R: Read + Seek> {
    archive: ZipArchive<R>,
    config: Box<[CaseConfig]>,
}

pub type CaseResult<T> = Result<T, CaseError>;

#[derive(Debug)]
pub enum CaseError {
    Io(io::Error),
    InvalidArchive,
    FileNotFound,
    ParseError,
}

const DEFAULT_MEMORY: usize = 268435456;

struct CaseConfig {
    input_name: String,
    output_name: String,
    time: Duration,
    memory: usize,
    score: i32,
}

impl<R: Read + Seek> CaseVec<R> {
    pub fn load(package: R) -> CaseResult<CaseVec<R>> {
        let mut archive = ZipArchive::new(package)?;
        let mut canonical_names = HashMap::new();
        for index in 0..archive.len() {
            let file = archive.by_index(index)?;
            canonical_names.insert(file.name().to_ascii_lowercase(),
                                   file.name().to_string());
        }
        let config = match canonical_names.get("config.ini") {
            Some(name) =>
                parse_legacy_config(archive.by_name(name)?, &canonical_names)?,
            None => match canonical_names.get("config.yaml") {
                Some(_) => panic!("not implemented"),
                None => return Err(CaseError::FileNotFound),
            },
        };
        Ok(CaseVec { archive, config })
    }

    pub fn len(&self) -> usize {
        self.config.len()
    }
}

fn parse_legacy_config<R: Read>(
    config: R,
    canonical_names: &HashMap<String, String>,
) -> CaseResult<Box<[CaseConfig]>> {
    let mut lines = BufReader::new(config).lines();
    let num_cases = lines.next().ok_or(CaseError::ParseError)??.parse()?;
    let mut configs = Vec::with_capacity(num_cases);
    for _ in 0..num_cases {
        let line = lines.next().ok_or(CaseError::ParseError)??;
        let mut parts = line.split('|');
        let input_lowercase = format!(
            "input/{}",
            parts.next().ok_or(CaseError::ParseError)?.to_ascii_lowercase());
        let input_name = canonical_names.get(&input_lowercase)
            .ok_or(CaseError::FileNotFound)?.to_string();
        let output_lowercase = format!(
            "output/{}",
            parts.next().ok_or(CaseError::ParseError)?.to_ascii_lowercase());
        let output_name = canonical_names.get(&output_lowercase)
            .ok_or(CaseError::FileNotFound)?.to_string();
        let time_sec: f64 =
            parts.next().ok_or(CaseError::ParseError)?.parse()?;
        let time_nanos: u64 = (time_sec * 1e9) as u64;
        let time = Duration::new(time_nanos / 1_000_000_000,
                                 (time_nanos % 1_000_000_000) as u32);
        let score: i32 = parts.next().ok_or(CaseError::ParseError)?.parse()?;
        let memory =
            match parts.next().ok_or(CaseError::ParseError)?.parse::<f64>() {
                Ok(memory_kb) => (memory_kb * 1024.) as usize,
                Err(_) => DEFAULT_MEMORY,
            };
        let config =
            CaseConfig { input_name, output_name, time, memory, score };
        configs.push(config)
    }
    Ok(configs.into_boxed_slice())
}

impl From<io::Error> for CaseError {
    fn from(e: io::Error) -> CaseError {
        CaseError::Io(e)
    }
}

impl From<ZipError> for CaseError {
    fn from(e: ZipError) -> CaseError {
        match e {
            ZipError::Io(e) => CaseError::Io(e),
            ZipError::InvalidArchive(_) | ZipError::UnsupportedArchive(_) =>
                CaseError::InvalidArchive,
            ZipError::FileNotFound => CaseError::FileNotFound,
        }
    }
}

impl From<ParseFloatError> for CaseError {
    fn from(_: ParseFloatError) -> CaseError {
        CaseError::ParseError
    }
}

impl From<ParseIntError> for CaseError {
    fn from(_: ParseIntError) -> CaseError {
        CaseError::ParseError
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn invalid_archive() {
        let reader = Cursor::new(&[]);
        assert!(match CaseVec::load(reader) {
            Err(CaseError::InvalidArchive) => true,
            _ => false,
        });
    }

    #[test]
    fn legacy_archive() {
        let data = include_bytes!("testdata/aplusb-legacy.zip");
        let reader = Cursor::new(&data[..]);
        let mut cases = CaseVec::load(reader).unwrap();
        assert_eq!(cases.len(), 10);
    }
}
