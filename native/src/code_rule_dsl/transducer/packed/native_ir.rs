use super::{
    Program,
    value::{CaptureProgram, ValueProgram},
};

// The native IR and its lowering from the word program, the compiler crate's
// (`rerail_compiler::native_ir`).
pub(super) use rerail_compiler::native_ir::*;

/// A program's native IR (`Structure`, the compiler crate's) and the matcher's value, capture
/// and output programs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeModule {
    pub structure: Structure,
    pub value_programs: Vec<ValueProgram>,
    pub capture_programs: Vec<CaptureProgram>,
    pub output_program: super::output::OutputProgram,
}

impl std::ops::Deref for NativeModule {
    type Target = Structure;
    fn deref(&self) -> &Structure {
        &self.structure
    }
}

impl std::ops::DerefMut for NativeModule {
    fn deref_mut(&mut self) -> &mut Structure {
        &mut self.structure
    }
}

pub(super) fn lower(program: &Program) -> Result<NativeModule, LoweringError> {
    let value_functions = u16::try_from(program.value_programs.len()).map_err(|_| LoweringError::TooManyInstructions)?;
    let capture_functions = u16::try_from(program.capture_programs.len()).map_err(|_| LoweringError::TooManyInstructions)?;
    Ok(NativeModule {
        structure: rerail_compiler::native_ir::lower(program, value_functions, capture_functions)?,
        value_programs: program.value_programs.clone(),
        capture_programs: program.capture_programs.clone(),
        output_program: program.output_program.clone(),
    })
}

// The block programs' native IR (`BlockNative`) is tier 1's, in its own crate.
pub(super) use rerail_compiler::block_native::*;

#[cfg(test)]
mod tests {
    use super::*;
    // G's tests (b90a7b0a), dropped by bb4c9605's merge; `policy_dsl::transducer` is `packed`.
    use super::super::{GREEDY, NAMED, Program, word};
    use rerail_compiler::program::*;

    #[test]
    fn native_ir_decodes_control_flow_without_a_host_call_operation() {
        let program = Program::new(
            vec![
                word(KIND, 0, 7),
                word(FORK, GREEDY, 4),
                word(CHILD, NAMED, 0),
                word(GOTO, 0, 4),
                word(EMIT, 0, 0),
            ],
            vec![b"unused".to_vec(), b"unused".to_vec(), b"match".to_vec()],
            Vec::new(),
            vec![2],
            Vec::new(),
        )
        .unwrap();
        let native = lower(&program).unwrap();
        assert_eq!(native.structural.len(), 5);
        assert_eq!(
            native.structural[1].operation,
            StructuralOperation::Fork {
                greedy: true,
                target: 4,
            }
        );
        assert_eq!(
            native.structural[2].operation,
            StructuralOperation::Child { named: true }
        );
        assert_eq!(native.structural[3].next, Some(4));
        assert_eq!(native.structural[4].next, None);
        assert!(!native.tree_sitter_query);
        assert!(!native.has_repeats);
        assert_eq!(native.maximum_cursor_slot, 0);
        assert!(native.reservation_sites.contains(&ReservationSite {
            arena: ArenaKind::Pending,
            strategy: ReservationStrategy::Chunked,
            origin: ReservationOrigin::Structural(1),
        }));
        assert!(native.reservation_sites.contains(&ReservationSite {
            arena: ArenaKind::Pending,
            strategy: ReservationStrategy::Chunked,
            origin: ReservationOrigin::Structural(2),
        }));
        assert!(native.reservation_sites.contains(&ReservationSite {
            arena: ArenaKind::Result,
            strategy: ReservationStrategy::Exact,
            origin: ReservationOrigin::Reduction,
        }));
        assert!(
            !native
                .reservation_sites
                .iter()
                .any(|site| site.arena == ArenaKind::QueryBucket)
        );
    }

    #[test]
    fn program_verification_rejects_an_invalid_branch_target_before_native_lowering() {
        let Err(error) = Program::new(
            vec![word(GOTO, 0, 2)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ) else {
            panic!("invalid target was accepted");
        };
        assert!(
            matches!(
                error,
                super::super::Error::InvalidTarget(_)
            ),
            "{error:?}"
        );
    }
}
