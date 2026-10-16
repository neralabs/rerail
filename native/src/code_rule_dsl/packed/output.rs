use super::{Directive, Error, value::ByteNfa};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OutputProgram {
    pub(super) instructions: Vec<OutputInstruction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OutputInstruction {
    MatchProperty {
        key: u16,
        value: Option<u16>,
    },
    CaptureProperty {
        capture: u16,
        key: u16,
        value: Option<u16>,
    },
    Offset {
        capture: u16,
        offsets: [i32; 4],
    },
    Strip {
        capture: u16,
        regex: ByteNfa,
    },
    SelectAdjacent {
        capture: u16,
        target: u16,
    },
    Downcase {
        capture: u16,
    },
    LanguageFromMime {
        capture: u16,
    },
}

impl OutputProgram {
    pub(super) fn lower(directives: &[Directive], strings: &[Vec<u8>]) -> Result<Self, Error> {
        let instructions = directives
            .iter()
            .map(|directive| {
                Ok(match directive {
                    Directive::Set { key, value } => OutputInstruction::MatchProperty {
                        key: *key,
                        value: *value,
                    },
                    Directive::SetCapture {
                        capture,
                        key,
                        value,
                    } => OutputInstruction::CaptureProperty {
                        capture: *capture,
                        key: *key,
                        value: *value,
                    },
                    Directive::Offset { capture, offsets } => OutputInstruction::Offset {
                        capture: *capture,
                        offsets: *offsets,
                    },
                    Directive::Strip {
                        capture, source, ..
                    } => OutputInstruction::Strip {
                        capture: *capture,
                        regex: ByteNfa::compile(
                            std::str::from_utf8(&strings[usize::from(*source)])
                                .map_err(|_| Error::InvalidOperand(usize::from(*source)))?,
                        )?,
                    },
                    Directive::SelectAdjacent { capture, target } => {
                        OutputInstruction::SelectAdjacent {
                            capture: *capture,
                            target: *target,
                        }
                    }
                    Directive::Downcase { capture } => {
                        OutputInstruction::Downcase { capture: *capture }
                    }
                    Directive::SetLanguageFromMimeType { capture } => {
                        OutputInstruction::LanguageFromMime { capture: *capture }
                    }
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self { instructions })
    }
}
