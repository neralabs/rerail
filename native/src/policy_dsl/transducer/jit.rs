use super::{Capture, Error, KIND, Output, Program, TEXT, split};
use crate::policy_dsl::tape::TapeDoc;

mod native {
    use super::*;
    use std::ffi::c_void;
    use std::mem;
    use std::ptr;

    type MatchFn = unsafe extern "C" fn(u64, *const u8, u64) -> u64;

    unsafe extern "C" {
        fn sys_icache_invalidate(start: *mut c_void, len: usize);
    }

    pub(crate) struct LiteralJit {
        address: *mut c_void,
        length: usize,
        function: MatchFn,
        capture: u16,
        output: u16,
    }

    unsafe impl Send for LiteralJit {}
    unsafe impl Sync for LiteralJit {}

    impl Drop for LiteralJit {
        fn drop(&mut self) {
            unsafe { libc::munmap(self.address, self.length) };
        }
    }

    fn branch_ne() -> u32 {
        0x5400_0001
    }

    impl LiteralJit {
        pub(super) fn compile(program: &Program) -> Result<Self, Error> {
            if program.words.len() != 4
                || !matches!(split(program.words[0]), (KIND, 0, _))
                || !matches!(split(program.words[1]), (TEXT, 0, _))
                || !matches!(split(program.words[2]), (super::super::CAPTURE, 0, _))
                || !matches!(split(program.words[3]), (super::super::EMIT, 0, _))
            {
                return Err(Error::UnsupportedBackend);
            }
            let (_, _, kind) = split(program.words[0]);
            let (_, _, predicate) = split(program.words[1]);
            let super::super::TextPredicate::Equal(string) =
                program.predicates[usize::from(predicate)]
            else {
                return Err(Error::UnsupportedBackend);
            };
            let expected = &program.strings[usize::from(string)];
            if kind > 4095 || expected.len() > 4095 {
                return Err(Error::UnsupportedBackend);
            }
            let mut code = Vec::<u32>::new();
            let mut failures = Vec::new();
            code.push(0x7100_001f | (u32::from(kind) << 10)); // cmp w0, #kind
            failures.push(code.len());
            code.push(branch_ne());
            code.push(0xf100_005f | ((expected.len() as u32) << 10)); // cmp x2, #length
            failures.push(code.len());
            code.push(branch_ne());
            for (offset, &byte) in expected.iter().enumerate() {
                code.push(0x3940_0023 | ((offset as u32) << 10)); // ldrb w3, [x1,#offset]
                code.push(0x7100_007f | (u32::from(byte) << 10)); // cmp w3, #byte
                failures.push(code.len());
                code.push(branch_ne());
            }
            code.extend([0x5280_0020, 0xd65f_03c0]); // mov w0,#1; ret
            let fail = code.len();
            code.extend([0x5280_0000, 0xd65f_03c0]); // mov w0,#0; ret
            for branch in failures {
                let delta = fail as isize - branch as isize;
                code[branch] |= ((delta as u32) & 0x7ffff) << 5;
            }
            let bytes = code.len() * 4;
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
            let length = bytes.div_ceil(page) * page;
            let address = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    length,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            if address == libc::MAP_FAILED {
                return Err(Error::BackendFailure);
            }
            unsafe {
                ptr::copy_nonoverlapping(code.as_ptr().cast::<u8>(), address.cast(), bytes);
                sys_icache_invalidate(address, bytes);
                if libc::mprotect(address, length, libc::PROT_READ | libc::PROT_EXEC) != 0 {
                    libc::munmap(address, length);
                    return Err(Error::BackendFailure);
                }
            }
            let function = unsafe { mem::transmute::<*mut c_void, MatchFn>(address) };
            Ok(Self {
                address,
                length,
                function,
                capture: split(program.words[2]).2,
                output: split(program.words[3]).2,
            })
        }

        fn matches(&self, tape: &TapeDoc, node: u32) -> bool {
            let text = tape.transducer_text_str(node).as_bytes();
            unsafe {
                (self.function)(
                    u64::from(tape.transducer_kind(node)),
                    text.as_ptr(),
                    text.len() as u64,
                ) != 0
            }
        }
    }

    pub(crate) fn compile(programs: &[Program]) -> Result<Vec<LiteralJit>, Error> {
        programs.iter().map(LiteralJit::compile).collect()
    }

    pub(crate) fn execute(
        compiled: &[LiteralJit],
        files: &[TapeDoc],
        anchors: &[Vec<Vec<u32>>],
    ) -> Result<Vec<Vec<Vec<Output>>>, Error> {
        if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != compiled.len()) {
            return Err(Error::InvalidAnchor);
        }
        files
            .iter()
            .enumerate()
            .map(|(file_index, tape)| {
                compiled
                    .iter()
                    .enumerate()
                    .map(|(program_index, program)| {
                        anchors[file_index][program_index]
                            .iter()
                            .map(|&node| {
                                if node as usize >= tape.transducer_len() {
                                    return Err(Error::InvalidAnchor);
                                }
                                if !program.matches(tape, node) {
                                    return Ok(None);
                                }
                                Ok(Some(Output {
                                    start: node,
                                    label: program.output,
                                    captures: vec![Capture {
                                        label: program.capture,
                                        node,
                                        span: tape.transducer_span(node),
                                    }],
                                }))
                            })
                            .filter_map(|result| result.transpose())
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }
}

pub(super) use native::{compile, execute};
