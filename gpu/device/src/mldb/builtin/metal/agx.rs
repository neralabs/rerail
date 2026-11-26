//! Apple G13's exit codes, read in a kernel's GPU code (decision 0672).
//!
//! The M1 family's back end (G13) joins a loop's exits that lead to different code (an early `return`, the normal
//! end) by numbering them: each exit writes its number into a 16-bit register (`mov_imm r4h, 299`), and after the
//! loop an `if_icmp`/`else_icmp` of that register against an immediate picks where to go. The immediate is 8 bits,
//! and the numbers count up through the whole compiled function. Past 255 the test compares against the number's
//! low byte (299 is tested as 43), no exit matches, and the thread leaves the function as if it had returned, with
//! whatever its registers hold. The M3 (G15) is not affected.
//!
//! So a G13 binary is read here: each function's instructions walked (their lengths from applegpu's tables,
//! `agx_g13_table.rs`), and every 16-bit `mov_imm` into a half register of a number past 255 whose low byte an
//! `if_icmp`/`else_icmp` of the same register tests is an exit the hardware will miss. The engine refuses such a
//! kernel (`compute_kernel_metal::refuse_exit_codes`); its functions must be split smaller.
//!
//! Plain bytes in, no Metal: it reads a binary from any machine.

#[path = "agx_g13_table.rs"]
mod table;

use std::collections::HashSet;
use std::sync::OnceLock;

/// A Mach-O's GPU CPU type (`CPU_TYPE_APPLE_GPU`, ABI64).
const AGX: u32 = 0x0100_0013;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;

/// A number past 255 that a 16-bit `mov_imm` puts in a function's exit register, its low byte tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    pub function: String,
    /// The half register, applegpu's numbering (`r4h` is 9).
    pub register: u8,
    pub code: u16,
}

/// What a binary's GPU code holds: its functions and instructions walked, and the exits past 255.
#[derive(Debug, Default)]
pub struct Exits {
    pub functions: usize,
    pub instructions: usize,
    /// Per function, its highest exit code (a number its exit register is set to whose low byte is tested).
    pub highest: Vec<(String, u16)>,
    pub wrapped: Vec<Wrapped>,
}

/// The highest exit code a G13 function's tests can tell apart.
pub const EXIT_CODES: u16 = 256;

/// Reads `binary` (a serialized dynamic library or binary archive: a fat file with a GPU slice, or a GPU Mach-O):
/// every function of its G13 code walked. An instruction the tables do not decode is an error: the function is
/// then not read, and not known to be right.
pub fn exits(binary: &[u8]) -> Result<Exits, String> {
    let mut images = Vec::new();
    gpu_slices(binary, &mut images)?;
    if images.is_empty() {
        return Err("no GPU code in the binary".into());
    }
    let mut out = Exits::default();
    for image in images {
        for (name, code) in image.functions() {
            walk(&name, code, &mut out)?;
        }
    }
    Ok(out)
}

/// A code section of a GPU Mach-O: its bytes and address, and its functions' names and addresses.
struct Image<'a> {
    text: &'a [u8],
    addr: u64,
    symbols: Vec<(u64, String)>,
}

impl<'a> Image<'a> {
    /// Each function's name and bytes: from its symbol to the next one (or the section's end); the whole section
    /// where it has no symbol.
    fn functions(&self) -> Vec<(String, &'a [u8])> {
        let end = self.addr + self.text.len() as u64;
        let mut starts: Vec<(u64, String)> = self.symbols.iter().filter(|(a, _)| *a >= self.addr && *a < end).cloned().collect();
        starts.sort();
        starts.dedup_by_key(|(a, _)| *a);
        if starts.is_empty() {
            return vec![("(the section)".into(), self.text)];
        }
        let mut out = Vec::new();
        for (i, (a, name)) in starts.iter().enumerate() {
            let next = starts.get(i + 1).map_or(end, |(b, _)| *b);
            out.push((name.clone(), &self.text[(a - self.addr) as usize..(next - self.addr) as usize]));
        }
        out
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn name16(b: &[u8], at: usize) -> Option<&str> {
    let s = b.get(at..at + 16)?;
    std::str::from_utf8(&s[..s.iter().position(|&c| c == 0).unwrap_or(16)]).ok()
}

/// The GPU Mach-Os of `b`: a fat file's GPU slices (Metal's fat magic `0xcbfebabe`, or the usual `0xcafebabe`), or
/// `b` itself.
fn gpu_slices<'a>(b: &'a [u8], out: &mut Vec<Image<'a>>) -> Result<(), String> {
    let magic = be32(b, 0).ok_or("a binary of under 4 bytes")?;
    if magic == 0xcbfe_babe || magic == 0xcafe_babe {
        let n = be32(b, 4).ok_or("a fat header cut short")? as usize;
        for i in 0..n {
            let e = 8 + 20 * i;
            let (cpu, off, size) = (be32(b, e).ok_or("a fat entry cut short")?, be32(b, e + 8).ok_or("a fat entry cut short")?, be32(b, e + 12).ok_or("a fat entry cut short")?);
            if cpu == AGX {
                let slice = b.get(off as usize..(off as usize).saturating_add(size as usize)).ok_or("a fat slice out of bounds")?;
                macho(slice, out)?;
            }
        }
        return Ok(());
    }
    if le32(b, 0) == Some(MH_MAGIC_64) {
        return macho(b, out);
    }
    Err(format!("not a Mach-O or fat binary (magic {magic:08x})"))
}

/// A GPU Mach-O's code: each section that holds a nested GPU Mach-O read as one (a dynamic library's `__text`, an
/// archive's `__compute`), and a `__text` that holds code read with the file's symbols.
fn macho<'a>(m: &'a [u8], out: &mut Vec<Image<'a>>) -> Result<(), String> {
    let cut = || "a GPU Mach-O cut short".to_string();
    if le32(m, 0) != Some(MH_MAGIC_64) || le32(m, 4) != Some(AGX) {
        return Ok(());
    }
    let ncmds = le32(m, 16).ok_or_else(cut)? as usize;
    let mut p = 32usize;
    let mut sections: Vec<(&str, u64, &'a [u8])> = Vec::new();
    let mut symtab = None;
    for _ in 0..ncmds {
        let (cmd, size) = (le32(m, p).ok_or_else(cut)?, le32(m, p + 4).ok_or_else(cut)? as usize);
        if size < 8 {
            return Err("a load command of under 8 bytes".into());
        }
        if cmd == LC_SEGMENT_64 {
            let nsects = le32(m, p + 64).ok_or_else(cut)? as usize;
            for s in 0..nsects {
                let q = p + 72 + 80 * s;
                let sect = name16(m, q).ok_or_else(cut)?;
                let (addr, len, off) = (le64(m, q + 32).ok_or_else(cut)?, le64(m, q + 40).ok_or_else(cut)? as usize, le32(m, q + 48).ok_or_else(cut)? as usize);
                if len == 0 || off == 0 {
                    continue;
                }
                let bytes = m.get(off..off.saturating_add(len)).ok_or("a section out of bounds")?;
                sections.push((sect, addr, bytes));
            }
        } else if cmd == LC_SYMTAB {
            symtab = Some((le32(m, p + 8).ok_or_else(cut)? as usize, le32(m, p + 12).ok_or_else(cut)? as usize, le32(m, p + 16).ok_or_else(cut)? as usize, le32(m, p + 20).ok_or_else(cut)? as usize));
        }
        p += size;
    }
    for &(sect, addr, bytes) in &sections {
        if le32(bytes, 0) == Some(MH_MAGIC_64) && le32(bytes, 4) == Some(AGX) {
            macho(bytes, out)?;
        } else if sect == "__text" {
            let mut symbols = Vec::new();
            if let Some((symoff, nsyms, stroff, strsize)) = symtab {
                let strings = m.get(stroff..stroff.saturating_add(strsize)).ok_or("a string table out of bounds")?;
                for i in 0..nsyms {
                    let e = symoff + 16 * i;
                    let (strx, value) = (le32(m, e).ok_or_else(cut)? as usize, le64(m, e + 8).ok_or_else(cut)?);
                    let name = strings.get(strx..).map(|s| String::from_utf8_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]).into_owned()).unwrap_or_default();
                    symbols.push((value, name));
                }
            }
            out.push(Image { text: bytes, addr, symbols });
        }
    }
    Ok(())
}

/// The encodings that can be an instruction with these low 7 bits (every encoding constrains them, or none).
fn by_opcode() -> &'static [Vec<u16>; 128] {
    static TABLE: OnceLock<[Vec<u16>; 128]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t: [Vec<u16>; 128] = std::array::from_fn(|_| Vec::new());
        for (i, e) in table::ENCODINGS.iter().enumerate() {
            for (op, list) in t.iter_mut().enumerate() {
                if (op as u128 & e.mask & 0x7f) == (e.bits & 0x7f) {
                    list.push(i as u16);
                }
            }
        }
        t
    })
}

fn field(w: u128, at: u32, n: u32) -> u32 {
    ((w >> at) & ((1u128 << n) - 1)) as u32
}

/// The 16-bit register an ALU source names (applegpu's `ALUSrcDesc`): its value, where its type says a 16-bit
/// register (the low two bits a hint, the next two 0).
fn src16(w: u128, at: u32, ex: u32) -> Option<u8> {
    let t = field(w, at + 6, 4);
    ((t & 0b11) != 0 && (t & 0b1100) == 0).then(|| (field(w, at, 6) | (field(w, ex, 2) << 6)) as u8)
}

/// The immediate an ALU source is (type 0): 8 bits.
fn imm8(w: u128, at: u32, ex: u32) -> Option<u8> {
    (field(w, at + 6, 4) == 0).then(|| (field(w, at, 6) | (field(w, ex, 2) << 6)) as u8)
}

/// Walks one function: its instructions decoded by length (applegpu's: zero padding skipped two bytes at a time, the
/// walk ended at `stop`), its exit registers' numbers and tests gathered.
fn walk(name: &str, code: &[u8], out: &mut Exits) -> Result<(), String> {
    let index = by_opcode();
    let mut sets: HashSet<(u8, u16)> = HashSet::new();
    let mut tests: HashSet<(u8, u8)> = HashSet::new();
    let mut p = 0usize;
    while p + 2 <= code.len() {
        if code.get(p..p + 4) == Some(&[0, 0, 0, 0]) {
            while code.get(p..p + 2) == Some(&[0, 0]) {
                p += 2;
            }
            continue;
        }
        let mut bytes = [0u8; 16];
        let take = (code.len() - p).min(12);
        bytes[..take].copy_from_slice(&code[p..p + take]);
        let w = u128::from_le_bytes(bytes);
        let found = index[(w & 0x7f) as usize].iter().map(|&i| &table::ENCODINGS[i as usize]).find_map(|e| {
            let len = e.lengths[field(w, e.length_bit as u32, 1) as usize] as usize;
            let cut = if len >= 16 { w } else { w & ((1u128 << (8 * len)) - 1) };
            (len <= take && (cut & e.mask) == e.bits).then_some((e, len, cut))
        });
        let Some((e, len, w)) = found else {
            return Err(format!("{name}: the instruction at {p:#x} ({}) is not one applegpu's tables decode", code[p..p + take.min(8)].iter().map(|b| format!("{b:02x}")).collect::<String>()));
        };
        out.instructions += 1;
        match e.name {
            // `mov_imm D, imm16`: D at bits 9..15 and 44..46, its type at 7..9 (bit 8 set: a 32-bit register).
            "MovImm16InstructionDesc" if field(w, 8, 1) == 0 => {
                let reg = (field(w, 9, 6) | (field(w, 44, 2) << 6)) as u8;
                // Half registers only (odd: `rNh`); a code past 0xfff is a constant (`0x3c00` is half 1.0).
                let v = field(w, 16, 16) as u16;
                if reg & 1 == 1 && v < 0x1000 {
                    sets.insert((reg, v));
                }
            }
            // `if_icmp`/`else_icmp r0l, cc, A, B`: cc at 13..16 and 8; A at 16 (ex 42), B at 28 (ex 40).
            "IfICmpInstructionDesc" | "ElseICmpInstructionDesc" if (field(w, 13, 3) | (field(w, 8, 1) << 3)) == 0 => {
                if let (Some(a), Some(b)) = (src16(w, 16, 42), imm8(w, 28, 40)) {
                    tests.insert((a, b));
                }
            }
            "StopInstructionDesc" => break,
            _ => {}
        }
        p += len;
    }
    out.functions += 1;
    let tested: Vec<(u8, u16)> = sets.into_iter().filter(|&(r, v)| tests.contains(&(r, v as u8))).collect();
    if let Some(&(_, high)) = tested.iter().max_by_key(|(_, v)| *v) {
        out.highest.push((name.to_owned(), high));
    }
    let mut wrapped: Vec<Wrapped> = tested.into_iter().filter(|&(_, v)| v >= EXIT_CODES).map(|(register, code)| Wrapped { function: name.to_owned(), register, code }).collect();
    wrapped.sort_by_key(|w| w.code);
    out.wrapped.extend(wrapped);
    Ok(())
}

/// The refusal of a binary whose exits wrap: each function, its exits past 255 and its highest.
pub fn refusal(what: &str, e: &Exits) -> String {
    let mut by: Vec<(&str, usize, u16)> = Vec::new();
    for w in &e.wrapped {
        match by.iter_mut().find(|(f, _, _)| *f == w.function) {
            Some(b) => {
                b.1 += 1;
                b.2 = b.2.max(w.code);
            }
            None => by.push((&w.function, 1, w.code)),
        }
    }
    let list: Vec<String> = by.iter().map(|(f, n, high)| format!("{f}: {n} exits past 255, the highest {high}")).collect();
    format!(
        "{what}: the Apple G13 (M1) back end numbers this kernel's loop exits past 255, which its 8-bit tests miss \
         (decision 0672); split the function smaller. {}",
        list.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GPU Mach-O of one `__text` section and one symbol at its start.
    fn image(code: &[u8]) -> Vec<u8> {
        let mut m = vec![0u8; 32];
        m[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        m[4..8].copy_from_slice(&AGX.to_le_bytes());
        m[16..20].copy_from_slice(&2u32.to_le_bytes());
        // LC_SEGMENT_64 with one section (72 + 80 bytes), then LC_SYMTAB (24 bytes).
        let text_off = 32 + 152 + 24;
        let sym_off = text_off + code.len();
        let str_off = sym_off + 16;
        let mut seg = vec![0u8; 152];
        seg[0..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
        seg[4..8].copy_from_slice(&152u32.to_le_bytes());
        seg[64..68].copy_from_slice(&1u32.to_le_bytes());
        seg[72..78].copy_from_slice(b"__text");
        seg[88..94].copy_from_slice(b"__TEXT");
        seg[104..112].copy_from_slice(&0u64.to_le_bytes());
        seg[112..120].copy_from_slice(&(code.len() as u64).to_le_bytes());
        seg[120..124].copy_from_slice(&(text_off as u32).to_le_bytes());
        m.extend(seg);
        let mut sym = vec![0u8; 24];
        sym[0..4].copy_from_slice(&LC_SYMTAB.to_le_bytes());
        sym[4..8].copy_from_slice(&24u32.to_le_bytes());
        sym[8..12].copy_from_slice(&(sym_off as u32).to_le_bytes());
        sym[12..16].copy_from_slice(&1u32.to_le_bytes());
        sym[16..20].copy_from_slice(&(str_off as u32).to_le_bytes());
        sym[20..24].copy_from_slice(&3u32.to_le_bytes());
        m.extend(sym);
        m.extend_from_slice(code);
        m.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        m.extend_from_slice(b"\0f\0");
        m
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The instructions of the M1 Max's miscompiled paste (lp_loop_run, 2026-11-24): exit 299 set, then tested as 43.
    #[test]
    fn an_exit_code_past_255_tested_by_its_low_byte_is_found() {
        let code = hex(concat!(
            "62122b01",     // mov_imm r4h, 299
            "529546a22403", // while_icmp r0l, nseq, r3, r101, 2
            "d21600000000", // pop_exec r0l.cache, 2
            "520849b00200", // if_icmp r0l, ueq, r4h, 43, 1
            "8800",         // stop
        ));
        let e = exits(&image(&code)).unwrap();
        assert_eq!((e.functions, e.instructions), (1, 5));
        assert_eq!(e.wrapped, vec![Wrapped { function: "f".into(), register: 9, code: 299 }]);
        assert!(refusal("k", &e).contains("f: 1 exits past 255, the highest 299"));
    }

    /// The same with the code under 256, and a half 1.0 (`0x3c00`) in the register whose low byte 0 is tested.
    #[test]
    fn codes_under_256_and_constants_are_not_exits_past_255() {
        let code = hex(concat!(
            "62122b00",     // mov_imm r4h, 43
            "6212003c",     // mov_imm r4h, 15360
            "520849b00200", // if_icmp r0l, ueq, r4h, 43, 1
            "520849000000", // if_icmp r0l, ueq, r4h, 0, 1
            "8800",
        ));
        let e = exits(&image(&code)).unwrap();
        assert!(e.wrapped.is_empty(), "{:?}", e.wrapped);
        assert_eq!(e.highest, vec![("f".into(), 43)]);
    }

    /// A reading aid: `RERAIL_AGX_BINARY=<file>[:<file>...]` prints each binary's functions' highest exit codes and
    /// the exits past 255 (a unit's `.metallib` in the kernel store, a binary archive).
    #[test]
    #[ignore]
    fn read_binaries() {
        for path in std::env::var("RERAIL_AGX_BINARY").expect("RERAIL_AGX_BINARY").split(':') {
            let started = std::time::Instant::now();
            let e = exits(&std::fs::read(path).unwrap()).unwrap();
            println!("{path}: {} functions, {} instructions, {} exits past 255 ({:.0} ms)", e.functions, e.instructions, e.wrapped.len(), started.elapsed().as_secs_f64() * 1e3);
            let mut high = e.highest.clone();
            high.sort_by_key(|(_, h)| std::cmp::Reverse(*h));
            for (f, h) in high.iter().take(4) {
                println!("  highest {h:4} {}", &f[..f.len().min(100)]);
            }
            if !e.wrapped.is_empty() {
                println!("  {}", refusal(path, &e).chars().take(400).collect::<String>());
            }
        }
    }

    /// Bytes no encoding decodes: the function is not known to be right.
    #[test]
    fn an_instruction_the_tables_do_not_decode_is_an_error() {
        let e = exits(&image(&hex("ffffffffffffffffffffffff"))).unwrap_err();
        assert!(e.contains("not one applegpu's tables decode"), "{e}");
    }
}
