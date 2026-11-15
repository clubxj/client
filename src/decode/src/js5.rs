use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const SECTOR: u64 = 520;
const INDEX_ENTRY: u64 = 6;
const REFERENCE: u8 = 255;
const MAX_GROUP: usize = 64 << 20;

pub const DATA_FILE: &str = "main_file_cache.dat2";

pub struct Store {
    dir: PathBuf,
    data: File,
}

fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> Option<()> {
    let mut done = 0;
    while done < buffer.len() {
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(
            file,
            &mut buffer[done..],
            offset + done as u64,
        );
        #[cfg(unix)]
        let read =
            std::os::unix::fs::FileExt::read_at(file, &mut buffer[done..], offset + done as u64);
        match read.ok()? {
            0 => return None,
            n => done += n,
        }
    }
    Some(())
}

fn be(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |value, &b| value << 8 | b as u32)
}

impl Store {
    pub fn open(dir: &Path) -> Option<Self> {
        Some(Self {
            dir: dir.to_path_buf(),
            data: File::open(dir.join(DATA_FILE)).ok()?,
        })
    }

    fn raw(&self, index: u8, group: u32) -> Option<Vec<u8>> {
        let idx = File::open(self.dir.join(format!("main_file_cache.idx{index}"))).ok()?;
        let mut entry = [0u8; INDEX_ENTRY as usize];
        read_at(&idx, &mut entry, group as u64 * INDEX_ENTRY)?;
        let size = be(&entry[..3]) as usize;
        let mut sector = be(&entry[3..]) as u64;
        if size > MAX_GROUP {
            return None;
        }
        let header = if group > 0xffff { 10 } else { 8 };
        let mut out = Vec::with_capacity(size);
        let mut block = [0u8; SECTOR as usize];
        for part in 0.. {
            if out.len() >= size {
                break;
            }
            if sector == 0 {
                return None;
            }
            read_at(&self.data, &mut block, sector * SECTOR)?;
            let (owner, at) = if header == 10 {
                (be(&block[..4]), 4)
            } else {
                (be(&block[..2]), 2)
            };
            let chunk = be(&block[at..at + 2]);
            let next = be(&block[at + 2..at + 5]) as u64;
            let owner_index = block[at + 5];
            if owner != group || chunk != part as u32 || owner_index != index {
                return None;
            }
            let take = (size - out.len()).min(SECTOR as usize - header);
            out.extend_from_slice(&block[header..header + take]);
            sector = next;
        }
        Some(out)
    }

    pub fn group(&self, index: u8, group: u32) -> Option<Vec<u8>> {
        container(&self.raw(index, group)?)
    }

    pub fn reference(&self, index: u8) -> Option<Reference> {
        reference(&self.group(REFERENCE, index as u32)?)
    }

    pub fn stored(&self, index: u8) -> Option<Vec<bool>> {
        let mut idx = Vec::new();
        File::open(self.dir.join(format!("main_file_cache.idx{index}")))
            .ok()?
            .read_to_end(&mut idx)
            .ok()?;
        Some(
            idx.as_chunks::<{ INDEX_ENTRY as usize }>()
                .0
                .iter()
                .map(|entry| be(&entry[..3]) != 0 && be(&entry[3..]) != 0)
                .collect(),
        )
    }

    pub fn files(&self, index: u8, group: u32) -> Option<Vec<(u32, Vec<u8>)>> {
        let reference = self.reference(index)?;
        split(&self.group(index, group)?, reference.files(group)?)
    }
}

pub fn container(bytes: &[u8]) -> Option<Vec<u8>> {
    let compression = *bytes.first()?;
    let length = be(bytes.get(1..5)?) as usize;
    if compression == 0 {
        return bytes.get(5..5 + length).map(<[u8]>::to_vec);
    }
    let size = be(bytes.get(5..9)?) as usize;
    let data = bytes.get(9..9 + length)?;
    if size > MAX_GROUP {
        return None;
    }
    let mut out = Vec::with_capacity(size);
    let limit = size as u64 + 1;
    match compression {
        1 => {
            let mut stream = b"BZh1".to_vec();
            stream.extend_from_slice(data);
            bzip2::read::BzDecoder::new(stream.as_slice())
                .take(limit)
                .read_to_end(&mut out)
                .ok()?;
        }
        2 => {
            flate2::read::GzDecoder::new(data)
                .take(limit)
                .read_to_end(&mut out)
                .ok()?;
        }
        _ => return None,
    }
    (out.len() == size).then_some(out)
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let bytes = self.data.get(self.at..self.at + n)?;
        self.at += n;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.bytes(4).map(be)
    }

    fn smart(&mut self, big: bool) -> Option<u32> {
        if big && *self.data.get(self.at)? & 0x80 != 0 {
            return self.u32().map(|v| v & 0x7fff_ffff);
        }
        self.bytes(2).map(be)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(drop)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub groups: Vec<(u32, Vec<u32>)>,
    pub fingerprint: u64,
}

impl Reference {
    pub fn files(&self, group: u32) -> Option<&[u32]> {
        self.groups
            .binary_search_by_key(&group, |(id, _)| *id)
            .ok()
            .map(|at| self.groups[at].1.as_slice())
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

pub fn reference(data: &[u8]) -> Option<Reference> {
    let mut r = Cursor { data, at: 0 };
    let protocol = r.u8()?;
    if protocol >= 6 {
        r.u32()?;
    }
    let flags = r.u8()?;
    let big = protocol >= 7;
    let count = r.smart(big)? as usize;
    let mut ids = Vec::with_capacity(count.min(data.len()));
    let mut id = 0u32;
    for _ in 0..count {
        id = id.wrapping_add(r.smart(big)?);
        ids.push(id);
    }
    let mut per_group = 4 + if flags & 1 != 0 { 4 } else { 0 };
    if flags & 8 != 0 {
        per_group += 4;
    }
    if flags & 2 != 0 {
        per_group += 64;
    }
    if flags & 4 != 0 {
        per_group += 8;
    }
    r.skip(count * (per_group + 4))?;
    let sizes = (0..count)
        .map(|_| r.smart(big).map(|n| n as usize))
        .collect::<Option<Vec<_>>>()?;
    let mut groups = Vec::with_capacity(count);
    for (&group, &n) in ids.iter().zip(&sizes) {
        let mut file = 0u32;
        let files = (0..n)
            .map(|_| {
                file = file.wrapping_add(r.smart(big)?);
                Some(file)
            })
            .collect::<Option<Vec<_>>>()?;
        groups.push((group, files));
    }
    groups.sort_unstable_by_key(|(id, _)| *id);
    Some(Reference {
        groups,
        fingerprint: fnv(data),
    })
}

pub fn split(data: &[u8], ids: &[u32]) -> Option<Vec<(u32, Vec<u8>)>> {
    if ids.len() == 1 {
        return Some(vec![(ids[0], data.to_vec())]);
    }
    let chunks = *data.last()? as usize;
    let table = data.len().checked_sub(1 + chunks * ids.len() * 4)?;
    let mut sizes = Cursor {
        data: &data[table..],
        at: 0,
    };
    let mut files: Vec<(u32, Vec<u8>)> = ids.iter().map(|&id| (id, Vec::new())).collect();
    let mut at = 0usize;
    for _ in 0..chunks {
        let mut size = 0i32;
        for file in files.iter_mut() {
            size = size.wrapping_add(sizes.u32()? as i32);
            let end = at.checked_add(usize::try_from(size).ok()?)?;
            file.1.extend_from_slice(data.get(at..end.min(table))?);
            at = end;
        }
    }
    Some(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn unpacks_every_compression() {
        let raw = b"definition bytes".to_vec();
        let mut none = vec![0];
        none.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        none.extend_from_slice(&raw);
        assert_eq!(container(&none), Some(raw.clone()));

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&raw).unwrap();
        let gz = gz.finish().unwrap();
        let mut gzip = vec![2];
        gzip.extend_from_slice(&(gz.len() as u32).to_be_bytes());
        gzip.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        gzip.extend_from_slice(&gz);
        assert_eq!(container(&gzip), Some(raw.clone()));

        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
        bz.write_all(&raw).unwrap();
        let bz = bz.finish().unwrap();
        let mut bzip = vec![1];
        bzip.extend_from_slice(&((bz.len() - 4) as u32).to_be_bytes());
        bzip.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        bzip.extend_from_slice(&bz[4..]);
        assert_eq!(container(&bzip), Some(raw));
    }

    #[test]
    fn stops_inflating_past_the_stated_size() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![0u8; 16 << 20]).unwrap();
        let gz = gz.finish().unwrap();
        let mut gzip = vec![2];
        gzip.extend_from_slice(&(gz.len() as u32).to_be_bytes());
        gzip.extend_from_slice(&16u32.to_be_bytes());
        gzip.extend_from_slice(&gz);
        assert_eq!(container(&gzip), None);
    }

    #[test]
    fn refuses_a_group_count_longer_than_the_table() {
        assert!(super::reference(&[7, 0, 0, 0, 0, 0, 0x7f, 0xff, 0xff, 0xff]).is_none());
    }

    #[test]
    fn splits_chunked_archives() {
        let mut data = b"ABCDEFG".to_vec();
        data.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1]);
        data.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 0]);
        data.push(2);
        assert_eq!(
            split(&data, &[3, 9]),
            Some(vec![(3, b"ADE".to_vec()), (9, b"BCFG".to_vec())])
        );
    }

    #[test]
    fn reads_the_files_of_each_group() {
        let mut reference = vec![7, 0, 0, 0, 1, 0];
        reference.extend_from_slice(&[0, 2, 0, 4, 0, 2]);
        reference.extend_from_slice(&[0; 16]);
        reference.extend_from_slice(&[0, 1, 0, 2]);
        reference.extend_from_slice(&[0, 5]);
        reference.extend_from_slice(&[0, 0, 0, 3]);
        let reference = super::reference(&reference).unwrap();
        assert_eq!(reference.files(6), Some(&[0, 3][..]));
        assert_eq!(reference.files(4), Some(&[5][..]));
        assert_eq!(reference.files(5), None);
    }

    #[test]
    fn reads_the_installed_cache() {
        let Some(dir) = std::env::var_os("VOLCREN_CACHE_DIR") else {
            eprintln!("skipped, VOLCREN_CACHE_DIR is not set");
            return;
        };
        let start = std::time::Instant::now();
        let store = Store::open(Path::new(&dir)).unwrap();
        let locs = store.files(2, 6).unwrap();
        let varbits = store.files(2, 14).unwrap();
        let decoded = locs
            .iter()
            .filter(|(_, data)| crate::definitions::decode_loc_text(data).is_some())
            .count();
        let named = |id: u32| {
            let data = &locs.iter().find(|(i, _)| *i == id).unwrap().1;
            crate::definitions::decode_loc_text(data).unwrap().1.name
        };
        println!(
            "{} locs, {decoded} decoded, {} varbits, {:?}; 18774 is {:?}",
            locs.len(),
            varbits.len(),
            start.elapsed(),
            named(18774)
        );
        assert!(decoded * 1000 >= locs.len() * 999);
    }
}
