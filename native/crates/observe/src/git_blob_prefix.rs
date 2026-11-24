//! Read a Git blob prefix without allocating its full payload or expanding a delta's whole base.
use super::Storage;
use gix::odb::pack::data::{Entry, entry::Header};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
};

struct Inflate {
    input: BufReader<File>,
    state: gix::zlib::Decompress,
}

impl Read for Inflate {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        gix::zlib::stream::inflate::read(&mut self.input, &mut self.state, out)
    }
}

impl Drop for Inflate {
    fn drop(&mut self) {
        rerail_prof::count(
            "git.prefix_inflated_bytes",
            self.state.total_out().min(usize::MAX as u64) as usize,
        );
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(PartialOrd, Ord, PartialEq, Eq, Clone)]
enum Location {
    Loose(PathBuf),
    Packed(PathBuf, u64),
}

/// Copy `length` bytes at `start` in this object to `out` in the caller's prefix.
struct Slice {
    start: u64,
    length: usize,
    out: usize,
}
impl Slice {
    fn end(&self) -> u64 {
        self.start + self.length as u64
    }
}

impl Storage {
    fn locate_blob_part(&self, id: gix::ObjectId) -> io::Result<Location> {
        let hex = id.to_string();
        for root in &self.roots {
            let path = root.join(&hex[..2]).join(&hex[2..]);
            if path.try_exists()? {
                return Ok(Location::Loose(path));
            }
        }
        let sources = self
            .indices
            .as_ref()
            .and_then(|index| index.suppliers(id))
            .ok_or_else(|| invalid("cannot read Git pack index"))?;
        sources
            .into_iter()
            .next()
            .map(|(path, offset)| Location::Packed(path, offset))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("missing Git object {id}")))
    }

    pub(in super::super) fn blob_prefix(&self, id: gix::ObjectId, mut size: u64, limit: u64) -> io::Result<Vec<u8>> {
        let length = usize::try_from(size.min(limit)).map_err(|_| invalid("prefix exceeds address space"))?;
        if length == 0 {
            return Ok(Vec::new());
        }
        let mut output = vec![0; length];
        let mut slices = vec![Slice {
            start: 0,
            length,
            out: 0,
        }];
        let mut location = self.locate_blob_part(id)?;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(location.clone()) {
                return Err(invalid("cycle in Git delta chain"));
            }
            slices.sort_unstable_by_key(|s| s.start);
            let (path, offset) = match &location {
                Location::Loose(p) => (p, None),
                Location::Packed(p, at) => (p, Some(*at)),
            };
            let mut input = BufReader::new(File::open(path)?);
            let entry = if let Some(at) = offset {
                input.seek(SeekFrom::Start(at))?;
                Some(Entry::from_read(&mut input, at, id.as_slice().len())?)
            } else {
                None
            };
            let mut bytes = Inflate {
                input,
                state: Default::default(),
            };
            let Some(entry) = entry else {
                let expected = format!("blob {size}\0");
                let mut header = vec![0; expected.len()];
                bytes.read_exact(&mut header)?;
                if header != expected.as_bytes() {
                    return Err(invalid("invalid Git loose blob header"));
                }
                read_slices(&mut bytes, &slices, &mut output)?;
                break;
            };
            let mut bytes = bytes.take(entry.decompressed_size);
            let base = match entry.header {
                Header::Blob if entry.decompressed_size == size => {
                    read_slices(&mut bytes, &slices, &mut output)?;
                    break;
                }
                Header::OfsDelta { base_distance } => Location::Packed(
                    path.clone(),
                    Header::verified_base_pack_offset(offset.unwrap(), base_distance)
                        .ok_or_else(|| invalid("invalid Git delta offset"))?,
                ),
                Header::RefDelta { base_id } => self.locate_blob_part(base_id)?,
                _ => return Err(invalid("invalid Git blob type or size")),
            };
            let base_size = delta_size(&mut bytes)?;
            if delta_size(&mut bytes)? != size {
                return Err(invalid("invalid Git delta result size"));
            }
            slices = delta_slices(&mut bytes, &slices, &mut output, size, base_size)?;
            if slices.is_empty() {
                break;
            }
            location = base;
            size = base_size;
        }
        Ok(output)
    }
}

/// Merge overlapping base reads so the compressed stream only moves forward.
fn read_slices(input: &mut impl Read, mut slices: &[Slice], output: &mut [u8]) -> io::Result<()> {
    let mut position = 0;
    let mut scratch = Vec::new();
    while let Some(first) = slices.first() {
        let mut end = first.end();
        let mut count = 1;
        while count < slices.len() && slices[count].start <= end {
            end = end.max(slices[count].end());
            count += 1;
        }
        let skip = first.start - position;
        if io::copy(&mut input.by_ref().take(skip), &mut io::sink())? != skip {
            return Err(invalid("truncated Git blob"));
        }
        scratch.resize((end - first.start) as usize, 0);
        input.read_exact(&mut scratch)?;
        for s in &slices[..count] {
            let i = (s.start - first.start) as usize;
            output[s.out..s.out + s.length].copy_from_slice(&scratch[i..i + s.length]);
        }
        position = end;
        slices = &slices[count..];
    }
    Ok(())
}

fn byte(input: &mut impl Read) -> io::Result<u8> {
    let mut b = [0];
    input.read_exact(&mut b)?;
    Ok(b[0])
}

fn delta_size(input: &mut impl Read) -> io::Result<u64> {
    let mut result = 0;
    for shift in (0..64).step_by(7) {
        let b = byte(input)?;
        if shift == 63 && b > 1 {
            break;
        }
        result |= u64::from(b & 127) << shift;
        if b & 128 == 0 {
            return Ok(result);
        }
    }
    Err(invalid("invalid Git delta size"))
}

fn copy_number(input: &mut impl Read, mask: u8, count: u32) -> io::Result<u64> {
    let mut value = 0;
    for i in 0..count {
        if mask & (1 << i) != 0 {
            value |= u64::from(byte(input)?) << (8 * i);
        }
    }
    Ok(value)
}

/// Resolve literal bytes here. Every copy in a delta names the same base object.
fn delta_slices(
    input: &mut impl Read,
    mut slices: &[Slice],
    output: &mut [u8],
    size: u64,
    base_size: u64,
) -> io::Result<Vec<Slice>> {
    let mut base = Vec::new();
    let mut position: u64 = 0;
    let mut literal = [0; 127];
    while !slices.is_empty() {
        let op = byte(input)?;
        let (start, length) = if op & 128 != 0 {
            let start = copy_number(input, op, 4)?;
            let n = copy_number(input, op >> 4, 3)?;
            let n = if n == 0 { 65536 } else { n };
            if start + n > base_size {
                return Err(invalid("Git delta copy exceeds its base"));
            }
            (Some(start), n)
        } else {
            if op == 0 {
                return Err(invalid("invalid Git delta instruction"));
            }
            input.read_exact(&mut literal[..usize::from(op)])?;
            (None, u64::from(op))
        };
        let end = position
            .checked_add(length)
            .filter(|p| *p <= size)
            .ok_or_else(|| invalid("Git delta exceeds its result"))?;
        for s in slices.iter().take_while(|s| s.start < end) {
            let lo = s.start.max(position);
            let hi = s.end().min(end);
            if hi <= lo {
                continue;
            }
            let n = (hi - lo) as usize;
            let out = s.out + (lo - s.start) as usize;
            let offset = lo - position;
            if let Some(start) = start {
                base.push(Slice {
                    start: start + offset,
                    length: n,
                    out,
                });
            } else {
                output[out..out + n].copy_from_slice(&literal[offset as usize..offset as usize + n]);
            }
        }
        position = end;
        while slices.first().is_some_and(|s| s.end() <= position) {
            slices = &slices[1..];
        }
    }
    Ok(base)
}
