use super::{
    Error,
    metal_batch_abi::{
        LuaInstructionRecord, LuaSetMemberRecord, NfaRangeRecord, NfaStateRecord, ValueRecord,
    },
    value::{
        ByteNfa, LuaInstruction, LuaProgram, NfaState, ValueMatcher, ValueProgram, ValueSetMember,
    },
};

fn checked_u32(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::BackendFailure)
}

#[derive(Clone, Debug, Default)]
pub(super) struct PackedValueTables {
    pub values: Vec<ValueRecord>,
    pub nfa_states: Vec<NfaStateRecord>,
    pub nfa_data: Vec<u32>,
    pub nfa_ranges: Vec<NfaRangeRecord>,
    pub lua_instructions: Vec<LuaInstructionRecord>,
    pub lua_members: Vec<LuaSetMemberRecord>,
}

impl PackedValueTables {
    pub(super) fn with_unicode_word_data() -> Self {
        Self {
            nfa_data: unicode_word_data(),
            ..Self::default()
        }
    }

    fn push_lua_matcher(
        &mut self,
        matcher: &ValueMatcher,
    ) -> Result<(u32, u32, u32, u32, u32), Error> {
        Ok(match matcher {
            ValueMatcher::Any => (0, 0, 0, 0, 0),
            ValueMatcher::Byte(byte) => (1, u32::from(*byte), 0, 0, 0),
            ValueMatcher::Class { class, negated } => {
                (2, u32::from(*class), u32::from(*negated), 0, 0)
            }
            ValueMatcher::Set { members, negated } => {
                let start = checked_u32(self.lua_members.len())?;
                for member in members {
                    self.lua_members.push(match member {
                        ValueSetMember::Byte(byte) => LuaSetMemberRecord {
                            kind: 0,
                            first: u32::from(*byte),
                            ..LuaSetMemberRecord::default()
                        },
                        ValueSetMember::Range(first, second) => LuaSetMemberRecord {
                            kind: 1,
                            first: u32::from(*first),
                            second: u32::from(*second),
                            ..LuaSetMemberRecord::default()
                        },
                        ValueSetMember::Class { class, negated } => LuaSetMemberRecord {
                            kind: 2,
                            first: u32::from(*class),
                            negated: u32::from(*negated),
                            ..LuaSetMemberRecord::default()
                        },
                    });
                }
                (
                    3,
                    u32::from(*negated),
                    0,
                    start,
                    checked_u32(members.len())?,
                )
            }
        })
    }

    pub(super) fn push_regex(&mut self, nfa: &ByteNfa) -> Result<(), Error> {
        let state_start = checked_u32(self.nfa_states.len())?;
        for state in &nfa.states {
            let record = match state {
                NfaState::Bytes(ranges) => {
                    let data_start = checked_u32(self.nfa_ranges.len())?;
                    self.nfa_ranges
                        .extend(ranges.iter().map(|(start, end, target)| NfaRangeRecord {
                            start: u32::from(*start),
                            end: u32::from(*end),
                            target: *target,
                        }));
                    NfaStateRecord {
                        kind: 0,
                        data_start,
                        data_count: checked_u32(ranges.len())?,
                        next: 0,
                    }
                }
                NfaState::Look { kind, next } => NfaStateRecord {
                    kind: 1,
                    data_start: *kind as u32,
                    data_count: 0,
                    next: *next,
                },
                NfaState::Split(alternatives) => {
                    let data_start = checked_u32(self.nfa_data.len())?;
                    self.nfa_data.extend_from_slice(alternatives);
                    NfaStateRecord {
                        kind: 2,
                        data_start,
                        data_count: checked_u32(alternatives.len())?,
                        next: 0,
                    }
                }
                NfaState::Capture { next, .. } => NfaStateRecord {
                    kind: 3,
                    next: *next,
                    ..NfaStateRecord::default()
                },
                NfaState::Fail => NfaStateRecord {
                    kind: 4,
                    ..NfaStateRecord::default()
                },
                NfaState::Match => NfaStateRecord {
                    kind: 5,
                    ..NfaStateRecord::default()
                },
            };
            self.nfa_states.push(record);
        }
        self.values.push(ValueRecord {
            kind: 2,
            operand: nfa.start,
            auxiliary: nfa.anchored_start,
            state_start,
            state_count: checked_u32(nfa.states.len())?,
        });
        Ok(())
    }

    fn push_lua(&mut self, lua: &LuaProgram) -> Result<(), Error> {
        let state_start = checked_u32(self.lua_instructions.len())?;
        for instruction in &lua.items {
            let mut record = LuaInstructionRecord::default();
            match instruction {
                LuaInstruction::Atom {
                    matcher,
                    repetition,
                } => {
                    record.kind = 0;
                    record.first = u32::from(*repetition);
                    (
                        record.matcher_kind,
                        record.matcher_first,
                        record.matcher_second,
                        record.data_start,
                        record.data_count,
                    ) = self.push_lua_matcher(matcher)?;
                }
                LuaInstruction::Frontier(matcher) => {
                    record.kind = 1;
                    (
                        record.matcher_kind,
                        record.matcher_first,
                        record.matcher_second,
                        record.data_start,
                        record.data_count,
                    ) = self.push_lua_matcher(matcher)?;
                }
                LuaInstruction::Balanced { open, close } => {
                    record.kind = 2;
                    record.first = u32::from(*open);
                    record.second = u32::from(*close);
                }
                LuaInstruction::CaptureStart(slot) => {
                    record.kind = 3;
                    record.first = *slot;
                }
                LuaInstruction::CaptureEnd(slot) => {
                    record.kind = 4;
                    record.first = *slot;
                }
                LuaInstruction::PositionCapture(slot) => {
                    record.kind = 5;
                    record.first = *slot;
                }
                LuaInstruction::Backreference(slot) => {
                    record.kind = 6;
                    record.first = *slot;
                }
            }
            self.lua_instructions.push(record);
        }
        self.values.push(ValueRecord {
            kind: 3,
            operand: u32::from(lua.anchored)
                | (u32::from(lua.end_anchored) << 1)
                | (lua.capture_count << 2),
            state_start,
            state_count: checked_u32(lua.items.len())?,
            ..ValueRecord::default()
        });
        Ok(())
    }

    pub(super) fn push_program(&mut self, values: &[ValueProgram]) -> Result<u32, Error> {
        let start = checked_u32(self.values.len())?;
        for value in values {
            match value {
                ValueProgram::Equal { string } => self.values.push(ValueRecord {
                    kind: 0,
                    operand: u32::from(*string),
                    ..ValueRecord::default()
                }),
                ValueProgram::Prefix { string } => self.values.push(ValueRecord {
                    kind: 1,
                    operand: u32::from(*string),
                    ..ValueRecord::default()
                }),
                ValueProgram::Regex(nfa) => self.push_regex(nfa)?,
                ValueProgram::Lua(lua) => self.push_lua(lua)?,
            }
        }
        Ok(start)
    }
}

pub(super) fn unicode_word_data() -> Vec<u32> {
    let hir = regex_syntax::Parser::new()
        .parse(r"\w")
        .expect("Unicode word tables are enabled");
    let regex_syntax::hir::HirKind::Class(regex_syntax::hir::Class::Unicode(class)) = hir.kind()
    else {
        unreachable!("Unicode word syntax must lower to a Unicode class")
    };
    let mut data = Vec::with_capacity(1 + class.ranges().len() * 2);
    data.push(class.ranges().len() as u32);
    for range in class.ranges() {
        data.extend([range.start() as u32, range.end() as u32]);
    }
    data
}
